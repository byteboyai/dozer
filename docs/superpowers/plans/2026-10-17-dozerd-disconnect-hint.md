# dozerd 断线提示 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 运行中 dozerd 被 `kill -9`(或异常退出)后,应用宿主面板在数秒内给用户一条"dozerd 连接中断,正在重连",恢复后给一条"已恢复";暂时的抖动不打扰用户。

**Architecture:** `bytehost-panel` 的 `PanelState` 状态机新增两个与界面无关的副作用 `PanelEffect::ConnectionLost` / `ConnectionRestored`(只在"已建立过的订阅连续失败"时各发一次),并发新 tag;`dozer-app` 的适配层把它们映射成带 key 的 Toast。**不**写 `App.daemon_unavailable`。

**Tech Stack:** Rust;`bytehost-panel`(bytehost 仓库)、`dozer-app` 的 `host_adapters/app_host.rs`、`push_toast_keyed`。

**Spec:** `docs/superpowers/manual_tests/2026-10-16-bytehost-manual-acceptance.md`(验收 3.2 与其"原因与待裁决"行);报告原文预期见 `docs/superpowers/specs/2026-10-08-bytehost-a6f-acceptance-report.md`(`kill -9` 后 2s 内显示 dozerd 不可用)。用户已裁决(2026-10-17):**补一个断线 Toast**。

## Global Constraints

- **瞬时消息走 Toast,持久状态不进 Toast**(CLAUDE.md):本计划只把"连接中断 / 已恢复"这两次**状态转换**当作瞬时事件推 Toast;`App.daemon_unavailable` 只有"启动连不上、设置里停止、打开项目成功即清除"三处写入,**本计划不新增写入点**。
- 状态机保持**界面无关**:`bytehost-panel` 不出现 Toast/文案资源,效果只带语义(`ConnectionLost`/`ConnectionRestored`),文案与 key 在 dozer 适配层。
- wire 形状只追加不修改;本计划不改任何 wire 类型。
- 依赖一律 git tag;发版(合并 main、打 tag、推送)是对外动作,**每次先问用户**;dozer 与 digger 同日升级 bytehost(ADR),本计划只做 dozer,digger 另行。
- bytehost 的单元测试要 `--features server`/`--all-features` 才完整(见上一份计划的裁决)。

## Review Focus

1. **启动时连不上 dozerd** 的情形(订阅从未建立过)**不**发 `ConnectionLost`——那由 `daemon_unavailable` 徽标负责,不能再叠一条 Toast。
2. **一次抖动**(丢一次、随即重订阅成功)不发任何提示,也不发"已恢复"。
3. **持续断线**期间只发**一次** `ConnectionLost`(退避重试再失败多少次都不重复)。
4. 用户在设置里**主动停止 dozerd**(`daemon_unavailable == Some(_)`)时,订阅随之断开**不**再弹"连接中断"。
5. 断线后又恢复、再次断线:第二轮**重新**发 `ConnectionLost`(标志位必须在恢复时复位)。

---

## 设计说明(请阅读)

`bytehost-panel/src/host.rs` 模块文档写着"列表请求的传输层失败(连不上 dozerd)不清空已知的应用、不弹 Notice——**顶栏的 dozerd 徽标负责告知**"。验收 3.2 证明这个分工的前提不成立:徽标只在三处被写入,运行中被 kill 时没人写它,于是界面完全静默。用户选了 Toast(本计划)。**另一种同样可行的映射**是让适配层在 `ConnectionLost` 时给徽标一个"可恢复的断线"状态(`daemon_unavailable` 的第四个写入点,恢复时清除)——它更符合"持久状态留在原位"的项目约定,因为"dozerd 断着"本身是状态而不只是事件。因为状态机只发中立效果,**两种映射只差适配层几行**;本计划按用户的裁决做 Toast,并在 Task 1 的文档更新里把这条设计分歧写明,留给以后一句话切换。

---

## Task 1(bytehost): 状态机发出 `ConnectionLost` / `ConnectionRestored`

**Files:**
- Modify: `crates/bytehost-panel/src/host.rs`(`PanelEffect` 枚举约 81-100 行;`PanelState` 字段约 219 行;`PanelMessage::Subscribed` / `SubscriptionLost` 分支约 361-380 行;模块文档第 10-12 行)

**Interfaces:**
- Produces: `PanelEffect::ConnectionLost`、`PanelEffect::ConnectionRestored`(无字段;枚举已是 `PanelEffect<K>`,新增变体与 `K` 无关)。
- 行为:`PanelState` 新增私有字段 `was_live: bool`(订阅曾到达 `Live`)与 `connection_lost_announced: bool`。
  - `Subscribed`:`was_live = true`;若 `connection_lost_announced` → 复位并在返回的效果里**先**放 `ConnectionRestored`、再放原有的重拉效果。
  - `SubscriptionLost`:仅当 `was_live` 且这是**连续第二次**失败(沿用现有退避档位:`attempt >= 1`)且尚未宣布时,置 `connection_lost_announced = true` 并返回 `vec![ConnectionLost]`;其余情形返回空,与现状一致。

- [ ] **Step 1: 写失败测试**(同文件 `mod tests`,沿用已有的 `PanelState::<TestKey>::default()`、`slot(..)`、`t0 = Instant::now()` 写法):
  - `the_second_consecutive_loss_of_a_live_subscription_announces_connection_lost_once`:`subscribe_if_due` → `Subscribed` → `SubscriptionLost`(断言返回空)→ 到第 0 档重订阅 → 再 `SubscriptionLost`(断言 `== vec![PanelEffect::ConnectionLost]`)→ 再连续丢 5 次,断言每次都返回空(只宣布一次)。
  - `a_subscription_that_never_became_live_does_not_announce_a_loss`:从 `Pending` 起连续 `SubscriptionLost` 6 次,断言全部返回空。
  - `a_single_blip_is_not_announced_either_way`:`Subscribed` → `SubscriptionLost` → `Subscribed`,断言两次 `update` 返回的效果里都不含 `ConnectionLost`/`ConnectionRestored`。
  - `resubscribing_after_an_announced_loss_announces_restored_once_and_resets`:丢两次(已宣布)→ `Subscribed` 返回的效果 `[0] == PanelEffect::ConnectionRestored` 且之后含 `FetchList`;再 `Subscribed` 一次(无新丢失)不含 `ConnectionRestored`;随后再连丢两次,**重新**返回 `[ConnectionLost]`。
- [ ] **Step 2: 运行确认失败**:`cd crates/bytehost-panel && cargo test --lib -- connection_lost announces restored blip never_became_live` → 编译失败(变体未定义)。
- [ ] **Step 3: 实现**:按上面 **行为** 一节;`PanelEffect` 的文档注释写清"只发一次、恢复时复位"。**更新模块文档第 10-12 行**:把"顶栏的 dozerd 徽标负责告知"改成"启动连不上/设置里停止由 `daemon_unavailable` 徽标负责;运行中**已建立的订阅**连续失败由 `ConnectionLost`/`ConnectionRestored` 告知(宿主自行决定映射成 Toast 或徽标)"。
- [ ] **Step 4: 运行**:`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all --check` → 全绿;检查既有测试 `losing_the_subscription_goes_back_to_fast_polling_and_retries_with_backoff` 与"dozerd 重启后重订阅恢复 `Live`"那条未被打破(它们只丢 1 次或不比较返回值,若有比较则按新行为调整并在提交说明里点名)。
- [ ] **Step 5: Commit**:`git commit -am "feat(panel): announce a lost/restored app-change subscription as ConnectionLost/ConnectionRestored effects"`

---

## Task 2(bytehost): 发版 `v0.4.6`(需用户批准后执行)

- [ ] **Step 1:** 整仓 `cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features && bash scripts/check-deps.sh` → 全绿。
- [ ] **Step 2:** 给用户审阅分支;**批准后**合并 `main`;**再次批准后** `git tag -a v0.4.6 -m "..." && git push origin main v0.4.6`;`git ls-remote --tags origin v0.4.6` 有输出。

---

## Task 3(dozer): 适配层把效果映射成带 key 的 Toast

**Files:**
- Modify: `crates/dozer-app/src/host_adapters/app_host.rs`(`match effect` 里 `Effect::Notice { .. }` 同处新增两条分支);各 `crates/*/Cargo.toml` 的 bytehost tag(`v0.4.5` → `v0.4.6`)与 `Cargo.lock`;文案资源(与同目录已有的 `settings.apps.*`/`app.*` 键放在一起,按现有 `host_text(key, 英文兜底, locale)` 的做法加键)。

**Interfaces:**
- Produces: `fn connection_toast(effect: &Effect, daemon_unavailable: bool) -> Option<(toast::Level, &'static str /*i18n key*/, &'static str /*英文兜底*/)>`(纯函数,放在该文件内,便于不构造 `App` 就能测试)。映射:`ConnectionLost` 且 `!daemon_unavailable` → `(Warning, "app.host.connection_lost", "dozerd connection lost, reconnecting…")`;`ConnectionRestored` → `(Success, "app.host.connection_restored", "dozerd connection restored")`;其余 `None`。key 固定 `"app-host-connection"`(同 key 的"已恢复"会顶替"连接中断")。
- Consumes: `App::push_toast_keyed(scope, level, text, key)`(`app/app.rs`)与 `crate::i18n::host_text`。

- [ ] **Step 1: 升 tag**:`grep -rn 'tag = "v0.4.5"' crates/*/Cargo.toml` 全部改 `v0.4.6`,`cargo update -p bytehost-panel`(及其余 `bytehost-*`),审阅 lock 变动只含 bytehost 与已有依赖边。
- [ ] **Step 2: 写失败测试**(该文件 `#[cfg(test)]`):
  - `a_lost_connection_becomes_a_keyed_warning_toast`:`connection_toast(&Effect::ConnectionLost, false)` 的级别是 `Warning`、key 为 `app.host.connection_lost`。
  - `a_lost_connection_is_not_announced_when_dozerd_was_stopped_on_purpose`:同一效果、`daemon_unavailable = true` → `None`。
  - `a_restored_connection_is_a_success_toast_even_when_the_badge_is_on`:`ConnectionRestored`、两种 `daemon_unavailable` 取值都是 `Some((Success, "app.host.connection_restored", _))`。
  - `other_effects_do_not_toast`:`Effect::FetchList` → `None`。
- [ ] **Step 3: 运行确认失败**:`cargo test -p dozer-app host_adapters::app_host` → 编译失败。
- [ ] **Step 4: 实现** `connection_toast`,并在 `match effect` 里对两个新效果调用它、再 `app.push_toast_keyed(LOG, level, <经 host_text 取的文案>, "app-host-connection")`;`Effect` 其余分支不动。给 `connection_lost`/`connection_restored` 两个键补上中文资源。
- [ ] **Step 5: 全量验证**:`cargo build && cargo test -p dozer-app -p dozerd -p dozer-client && cargo clippy -p dozer-app --all-targets`(新代码文件不得有新告警)。
- [ ] **Step 6: Commit**:`git commit -am "feat(app-host): toast when the live dozerd subscription is lost and restored (bytehost v0.4.6)"`

---

## Task 4(dozer): 真 GUI 回归 + 文档回写

- [ ] **Step 1: 真 GUI**(隔离 `HOME=/tmp/dzh`;环境准备见记忆 `bytehost-manual-acceptance-2026-10-17`):起 dozerd + GUI 并让应用面板正常跑起来 → `kill -9` 隔离的 dozerd → 数秒内 GUI 日志(`dozer-app.log`,每条 Toast 都会写一条日志)出现一条 Warning 的"连接中断"(截图里 Toast 是点击穿透的独立原生窗口,能截到就截);重启 dozerd → 出现"已恢复"。再测:设置里**主动停止** dozerd 时**没有**"连接中断"。
- [ ] **Step 2: 回写文档**:`docs/superpowers/manual_tests/2026-10-16-bytehost-manual-acceptance.md` 的 3.2 行与"3.2 的原因与待裁决"行改为已修复并写明实测结果与耗时;`docs/superpowers/unfinished/2026-10-16-bytehost-remaining.md` 去掉该待裁决项。
- [ ] **Step 3: Commit**:`git commit -am "docs: record the dozerd disconnect hint fix and its GUI verification"`

---

## Self-Review

- **Spec 覆盖:** 3.2 的两半(断线提示 / 恢复)分别由 Task 1(状态机)与 Task 3(适配)覆盖;"启动连不上不重复"、"主动停止不弹"、"抖动不打扰"、"持续断线只一次"、"恢复后复位"对应 Review Focus 1–5,各有测试。
- **类型一致:** `PanelEffect::ConnectionLost/ConnectionRestored`(Task 1 定义)在 Task 3 的 `connection_toast(&Effect, bool)` 里被消费;`Effect` 是 dozer 侧对 `PanelEffect<AppSlot>` 的别名(与现有 `Effect::Notice` 同一个)。
- **风险点:** Task 1 对既有订阅测试的影响要在 Step 4 逐条确认;阈值"连续第二次失败"意味着从 `kill -9` 到提示约 2–4 秒(第 0 档退避 2 秒),比报告原文的"2 秒"略长,验收时按此记录。
