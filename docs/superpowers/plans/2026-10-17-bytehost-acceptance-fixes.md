# bytehost 验收发现问题修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修掉 2026-10-17 真实 GUI 手工验收发现的、已有明确根因的 bytehost/Dozer 缺陷:WebSocket 经网关握手失败、应用日志文字不跟主题色、设置页应用行按钮被裁切、静态样例被 CSP 拦、审批卡"来源"标签误导,并让"读日志请求是否停止"可被观测。

**Architecture:** 网关问题与样例/标签问题在 `bytehost` 仓库修并发新 tag(`v0.4.5`),`dozer` 再整体升到该 tag;日志配色与设置页布局只在 `dozer-app` 内改,不依赖新 tag。一个需要产品裁决的项(dozerd 断线提示)放在决策闸门任务里,裁决前不写代码。

**Tech Stack:** Rust、hyper 1.12(网关,`hyper::server::conn::http1`)、iced 0.14 / `iced_widget`、`byteui` 主题 token(`ColorTokens::byteboy2077()` / `byteboy2077_light()`)、cargo 测试。

**Spec:** `docs/superpowers/manual_tests/2026-10-16-bytehost-manual-acceptance.md`(尤其结果表与"发现"各行);其来源报告 `docs/superpowers/specs/2026-10-08-bytehost-a6e-acceptance-report.md`、`…a6f-acceptance-report.md`。根因证据:`hyper-1.12.0/src/proto/h1/conn.rs:708-720`(`enforce_version` 在 keep-alive 被禁用时给 `Connection` 头追加 `close`)与 `role.rs:848`(响应自带 `Connection: close` 即标记为最后一个响应)。

## Global Constraints

- **不放宽 CSP。** 静态应用 CSP 保持 `script-src 'self' 'wasm-unsafe-eval'`(`bytehost-apps/src/gateway/mod.rs:csp_for`);样例去适配 CSP,不是反过来。
- **wire 形状只追加不修改**(`AppRequest`/`AppReply`);失败带类别。本计划不改任何 wire 类型。
- **应用宿主逻辑只在 `bytehost` 仓库**,`dozer` 不得往 `crates/` 里放(CLAUDE.md)。
- **依赖一律用 git tag**;本地联调的 `[patch]` 只放不提交的 `.cargo/config.toml`;提交前 `git checkout Cargo.lock` 去掉本地 patch 带来的 `source =` 行变化。
- **dozer 与 digger 同日一起升 bytehost/byteui**(digger `Cargo.toml` 注释里的 ADR);本计划只做 dozer 侧,digger 另起。
- **日志统一走 `dozer_core::log_*!`**,瞬时消息走 Toast(`App::push_toast_keyed`),不要给面板加 `Option<String>` 的 notice 字段;`bytehost-*` 用裸 `tracing`(`bytehost::service` 来源)。
- **字体:** 日志文字用 `Font::default()`,不得改成等宽。
- **新 UI 优先复用统一组件**;设置页行布局用 iced 自带的 `Row::wrap()`,不手写换行逻辑。
- **plan 要在独立分支(worktree)执行,审阅后再合并**;`bytehost` 与 `dozer` 各一条分支 `fix/acceptance-2026-10-17`。**推送、打 tag 是对外动作,每一次都先问用户。**
- bytehost 的 UI 文案是写死的中文(`bytehost-panel/src/install.rs`),本计划沿用,不引入新的 i18n 键。

## Review Focus

1. 非 101 的每一种响应(403、421、404、302、200、`HEAD`)在改成 keep-alive 之后**仍然**回完就关连接;客户端发 `Connection: keep-alive` 也一样(原测试只覆盖一个 403)。
2. 上游回了 `Connection: upgrade` 但状态不是 101(如 426)时,网关**不能**因为"带 upgrade"而放过关连接。
3. `101` 之后隧道仍双向转发字节(只改头不能把隧道弄断)。
4. 深色、浅色两套 token 下,日志文字与日志背景的对比度都 ≥ 4.5:1;切主题后不需要重启。
5. 设置页应用行在最窄对话框宽度、四个按钮(停止/回滚/日志/卸载)加一条很长的失败原因时,所有按钮仍完整可见、可点。

---

## 范围说明

**本计划修:** 验收文档里已有明确根因的 6 项(见 Goal)。**不在本计划里(记录原因,避免重复讨论):**

| 观察 | 为什么不在这里 |
|---|---|
| hang 期间界面无"无响应"提示;crash 恢复后页面不重载而 hang 恢复后重载 | 是产品行为取舍,没有验收标准说它错;需要单独设计 |
| Crashed 页要等约 40 秒(退避 5 次才放弃) | 策略参数(`RestartPolicy::default`)问题,改之前要先定"用户能接受等多久" |
| 已安装应用的入口图标无区分、卸载重装后位置变 | 需要应用图标/名称的设计,属 bytehost-chrome 的 rail 设计 |
| 受管 Node 下载未校验摘要、`LocalDir` 拷贝敏感文件等 | 已在 `unfinished/2026-10-16-bytehost-remaining.md` §4 记为有意局限 |
| 6.3 双窗口隔离 | 依赖分离窗口功能(已推迟) |

---

## File Structure

| 仓库 | 文件 | 责任 |
|---|---|---|
| bytehost | `crates/bytehost-apps/src/gateway/mod.rs` | 去掉 `.keep_alive(false)`,新增 `close_after`;网关测试 |
| bytehost | `crates/bytehost-apps/tests/process_apps_live.rs` | `ws_echo` 增加响应头断言(`#[ignore]` 的真应用测试) |
| bytehost | `scripts/samples/i18n/{bilingual-live,partial-initial}/web/{index.html,app.js}` | 内联脚本挪到外部文件 |
| bytehost | `crates/bytehost-apps/tests/i18n_samples.rs` | 新增"静态样例不得有内联可执行脚本"门禁 |
| bytehost | `crates/bytehost-panel/src/install.rs` | 审批卡渠道标签 |
| bytehost | `crates/bytehost-apps/src/service.rs` | `Logs` 请求的 `debug` 日志 |
| dozer | `crates/dozer-app/src/app/view.rs`、`extensions/settings_apps_view.rs` | 日志文字配色、应用行布局 |
| dozer | `crates/*/Cargo.toml`、`Cargo.lock` | 升 bytehost tag |
| dozer | `docs/superpowers/manual_tests/…`、`docs/superpowers/unfinished/2026-10-16-bytehost-remaining.md` | 回写验收结果与状态 |

---

## Task 1(bytehost): 101 升级响应保持 `Connection: Upgrade`

**Files:**
- Modify: `crates/bytehost-apps/src/gateway/mod.rs`(`http1::Builder` 设置处约 369-372 行;`mod tests` 约 1105 行起的辅助函数旁)
- Modify: `crates/bytehost-apps/tests/process_apps_live.rs`(`ws_echo`,约 325-375 行)

**Interfaces:**
- Produces: `fn close_after<B>(resp: Response<B>) -> Response<B>`(私有,`gateway/mod.rs`):状态不是 `StatusCode::SWITCHING_PROTOCOLS` 时往响应头设 `connection: close`(覆盖已有值),101 原样返回。网关每条路径的响应(静态站点、反向代理、错误)都经它出站。
- Consumes: 现有测试辅助 `raw(port) -> TcpStream`、`read_until_closed(&mut TcpStream) -> usize`、`Req::get(..).cookie(..).send() -> Resp`、`Gateway::{start, add_upstream, launch_url}`。

- [ ] **Step 1: 写失败测试 `a_101_upgrade_keeps_connection_exactly_upgrade_and_still_tunnels_bytes`**

  新增测试辅助 `fn fake_ws_upstream() -> SocketAddr`:`std::net::TcpListener` 绑 `127.0.0.1:0`,线程里对每个连接读完请求头后回
  `HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: <任意>\r\n\r\n`,之后把读到的字节原样写回(单次回显即可)。
  测试:`Gateway::start` → `gw.add_upstream(&id, fake_ws_upstream())` → 用 `gw.launch_url(&id)` 换会话 cookie(与同文件里 `launch` 相关的现有测试同一做法)→ 用 `raw(port)` 发带 `Upgrade: websocket`、`Connection: Upgrade`、同源 `Origin`、`Cookie` 的握手 → 读到 `\r\n\r\n`。断言:
  - 状态行含 `101`;
  - 取所有名为 `connection` 的响应头值,按逗号拆、`trim`、转小写后 `== vec!["upgrade"]`(**不含 `close`**);
  - 握手后发 `b"ping"`,读回 `b"ping"`(隧道仍通)。

- [ ] **Step 2: 写失败测试 `every_non_101_response_still_closes_the_connection`**

  参数化遍历以下请求(各自用新连接):无 cookie 的 `GET /`(403)、错误 `Host` 的 `GET /`(421)、未知应用(404)、`GET <launch_url>`(302)、带 cookie 的 `GET /`(200)、带 cookie 的 `HEAD /`、以及**带 `Connection: keep-alive` 请求头**的 403 请求;再加一个上游回 `Connection: upgrade` 但状态 `426 Upgrade Required` 的反向代理请求。每个请求断言:响应含 `connection` 头且其 token 集合含 `close`;`read_until_closed` 在 3 秒内返回(连接确实被服务端关闭)。

- [ ] **Step 3: 运行,确认第一个失败**

  Run: `cargo test -p bytehost-apps gateway::tests::a_101_upgrade -- --nocapture`
  Expected: FAIL,`connection` 头 token 是 `["upgrade","close"]`(当前 `.keep_alive(false)` 的表现)。第二个测试此时应已通过。

- [ ] **Step 4: 实现**

  在 `gateway/mod.rs` 删除 `http.… .keep_alive(false)`,保留 `timer` 与 `header_read_timeout`;把服务闭包产出的每个 `reply` 先过 `close_after` 再 `Ok(...)`。更新该处注释:说明"keep-alive 是连接级开关,hyper 会给所有响应(含 101)追加 `close`;所以改成由我们对非 101 响应显式声明 `Connection: close`"。同时把原测试 `keep_alive_is_off_so_idle_connections_do_not_pile_up` 的名字与注释改成与新机制一致(断言不变)。

- [ ] **Step 5: 运行网关全部测试与 clippy**

  Run: `cargo test -p bytehost-apps gateway && cargo clippy -p bytehost-apps --all-targets -- -D warnings`
  Expected: 全部通过,含原有 `connections_beyond_the_cap_are_dropped_immediately`、半截请求头超时测试(`header_read_timeout` 仍有效)。

- [ ] **Step 6: 加强真应用测试 `ws_echo`**

  在 `tests/process_apps_live.rs` 的 `ws_echo` 里,`status == 101` 之后对 `head_str` 追加断言:小写后包含 `connection: upgrade` 且**不包含** `upgrade, close`。
  Run: `cargo test -p bytehost-apps --test process_apps_live -- --ignored python_sample_end_to_end`(需要本机 `python3`)
  Expected: PASS。

- [ ] **Step 7: Commit**

  ```bash
  git add crates/bytehost-apps/src/gateway/mod.rs crates/bytehost-apps/tests/process_apps_live.rs
  git commit -m "fix(gateway): keep Connection: Upgrade on 101 — declare close per response instead of keep_alive(false)"
  ```

---

## Task 2(bytehost): 静态样例去掉内联脚本 + 门禁

**Files:**
- Modify: `scripts/samples/i18n/bilingual-live/web/index.html`、`scripts/samples/i18n/partial-initial/web/index.html`
- Create: 同目录下的 `web/app.js`(各一份)
- Modify: `crates/bytehost-apps/tests/i18n_samples.rs`

**Interfaces:**
- Produces: `fn static_samples_have_no_inline_executable_scripts()`(测试),遍历 `samples_dir()` 下每个 `*/web/*.html`。

- [ ] **Step 1: 写失败的门禁测试 `static_samples_have_no_inline_executable_scripts`**

  复用文件里的 `samples_dir()`;递归找 `scripts/samples` 下所有 `web/*.html`,对每个文件用不区分大小写的扫描找 `<script` 标签,标签里既没有 `src=` 也不是 `type="application/json"`(数据块不执行,CSP 不拦)的算违规。断言违规列表为空,失败信息列出 `文件:行号`。同时写一个只对内存字符串的小测试 `inline_script_scan_flags_only_executable_inline_scripts`,断言:`<script>x</script>` 违规;`<script src="app.js"></script>`、`<script type="application/json">{}</script>` 不违规;大小写 `<SCRIPT>` 违规。

- [ ] **Step 2: 运行,确认门禁测试因两个样例失败**

  Run: `cargo test -p bytehost-apps --test i18n_samples static_samples_have_no_inline`
  Expected: FAIL,点名 `bilingual-live/web/index.html` 与 `partial-initial/web/index.html`。

- [ ] **Step 3: 修样例**

  两个样例各自把 `<script>…</script>` 里的内容原样搬到 `web/app.js`,页面里改成 `<script src="app.js"></script>`(位置不变,仍在 `</body>` 前)。**脚本内容不改。**

- [ ] **Step 4: 运行样例相关测试**

  Run: `cargo test -p bytehost-apps --test i18n_samples`
  Expected: 全部通过(原有 `bilingual_live_is_live_and_resolves_all_locales` 等不受影响,它们只读 manifest 与 locales)。

- [ ] **Step 5: Commit**

  ```bash
  git add scripts/samples/i18n crates/bytehost-apps/tests/i18n_samples.rs
  git commit -m "fix(samples): move i18n samples' inline scripts to app.js so the strict CSP does not block them; add guard"
  ```

---

## Task 3(bytehost): 审批卡"来源渠道"标签

**Files:** Modify: `crates/bytehost-panel/src/install.rs`(`provenance_label` 约 877 行;事实行约 1076 行;测试约 1659、2343、2353 行)

**Interfaces:** 改 `provenance_label(p: Provenance) -> &'static str` 的返回值与审批卡事实行的项名;函数签名不变。

- [ ] **Step 1: 改测试**:`the_plan_view_shows_every_requested_change_with_its_real_enforcement` 里把 `("来源", "本地目录")` 改成 `("来源渠道", "本机")`;新增 `provenance_row_is_not_confused_with_the_source_block`:对一个 `source_info` 是压缩包的 plan 调 `plan_view`,断言 `facts` 中恰有一行项名为 `来源渠道`、其值**不含"目录"**,且来源区块那一行(`("来源","本机压缩包")`)仍在。
- [ ] **Step 2: 运行确认失败**:`cargo test -p bytehost-panel install::tests::provenance_row` → FAIL。
- [ ] **Step 3: 实现**:`provenance_label` 三个分支改为 `Local => "本机"`、`AgentGenerated => "Agent 生成"`、`ThirdParty => "第三方来源"`(后两项不变);事实行项名由 `"来源"` 改 `"来源渠道"`。**不动**来源区块的 `"来源"` 行。
- [ ] **Step 4: 运行**:`cargo test -p bytehost-panel && cargo clippy -p bytehost-panel --all-targets -- -D warnings` → 通过。
- [ ] **Step 5: Commit**:`git commit -am "fix(panel): label the provenance fact '来源渠道' so archive installs are not shown as '本地目录'"`

---

## Task 4(bytehost): `Logs` 请求的可观测性

**Files:** Modify: `crates/bytehost-apps/src/service.rs`(`AppRequest::Logs` 分支约 365 行)

**Interfaces:** 无签名变化;新增一条 `tracing::debug!(target: "bytehost::service", app = %id, max_lines, "读取应用日志")`。

- [ ] **Step 1: 写测试 `a_logs_request_leaves_a_debug_trace`**:用 `tracing_subscriber` 的测试 writer(`tracing-subscriber` 若不在 dev-dependencies 则加)把 `bytehost::service=debug` 的输出收进缓冲,对同文件现有 `AppRequest::Logs` 测试用的 `svc` 发一次 `Logs`,断言缓冲里含 `读取应用日志` 与应用 id;再发一次 `Stop`,断言**不含**该字样(避免过宽)。
- [ ] **Step 2: 运行确认失败**:`cargo test -p bytehost-apps service::tests::a_logs_request` → FAIL。
- [ ] **Step 3: 实现**:在 `Logs` 分支里先打这条 `debug!`,再走原逻辑。**必须是 `debug` 级**:GUI 每秒一次读取,`info` 会刷屏。
- [ ] **Step 4: 运行**:`cargo test -p bytehost-apps service && cargo clippy -p bytehost-apps --all-targets -- -D warnings` → 通过。
- [ ] **Step 5: Commit**:`git commit -am "feat(service): debug trace for Logs requests so 'log polling stops after closing' is observable"`

---

## Task 5(bytehost): 发版 `v0.4.5`(需用户批准后执行)

**Files:** Modify: `README.md`(若其中示例 tag 写了具体版本则同步)

- [ ] **Step 1:** `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` → 全绿。
- [ ] **Step 2:** 把分支给用户审阅;**得到批准后**合并 `main`。
- [ ] **Step 3:** 再次征得批准后 `git tag v0.4.5 && git push origin main v0.4.5`。验证:`git ls-remote --tags origin v0.4.5` 有输出。

---

## Task 6(dozer): 应用日志文字使用主题色

**Files:**
- Modify: `crates/dozer-app/src/app/view.rs`(`render_logs_body` 约 1422-1470 行,`iced_widget::text(text)` 处约 1450 行)
- Modify: `crates/dozer-app/src/extensions/settings_apps_view.rs`(`log_viewer` 约 328 行,`iced_widget::text(log_text)` 处约 367 行)

**Interfaces:**
- Produces: `pub(crate) fn log_text_color(tokens: &byteui::theme::color::ColorTokens) -> iced::Color`(放 `extensions/app_logs.rs`,两个调用点共用),返回 `tokens.cream`。
- Consumes: `byteui::theme::color::{ColorTokens::byteboy2077, ColorTokens::byteboy2077_light, current}`。

- [ ] **Step 1: 写失败测试**(`extensions/app_logs.rs`):
  - `log_text_is_readable_on_the_panel_in_both_themes`:对 `byteboy2077()` 与 `byteboy2077_light()` 各自断言 `contrast(log_text_color(&t), t.panel) >= 4.5`(在测试里写一个 WCAG 相对亮度对比度小函数)。
  - `the_two_log_views_use_the_shared_colour`:读两个源文件文本,断言各含 `log_text_color(`(防止以后又有一处漏设色)。
- [ ] **Step 2: 运行确认失败**:`cargo test -p dozer-app extensions::app_logs` → FAIL(函数未定义)。
- [ ] **Step 3: 实现** `log_text_color`;在两个日志 `text(...)` 上链上 `.color(log_text_color(&byteui::theme::color::current()))`(`render_logs_body` 只收到 `dim: Color`,没有整套 token,所以两处都在函数内直接取 `byteui::theme::color::current()`,不要为此给函数加参数)。字体与 `Shaping::Advanced` 保持不变。
- [ ] **Step 4: 运行**:`cargo test -p dozer-app extensions::app_logs` → PASS;`cargo clippy -p dozer-app --all-targets` 无新告警。
- [ ] **Step 5: 目视验证**(用 `orca computer`,环境准备见记忆 `bytehost-manual-acceptance-2026-10-17`):隔离 HOME 起 GUI,装 `py-crashy`,深色与浅色主题各截一张"查看日志"展开图,日志文字清晰可读。
- [ ] **Step 6: Commit**:`git add … && git commit -m "fix(app-host): app log text uses the theme's text colour (was iced's default, unreadable on dark)"`

---

## Task 7(dozer): 设置页应用行按钮不再被裁切

**Files:** Modify: `crates/dozer-app/src/extensions/settings_apps_view.rs`(`app_row` 约 201-300 行)

**Interfaces:** `fn app_row<'a>(state: &'a State, app: &'a AppSummary, locale: &str) -> El<'a>` 签名不变。结构改为两层:第一行 = 标题 + 版本/状态文字(可换行的 `text`);第二行 = 动作按钮组,用 `iced_widget::row![…].spacing(8).wrap()` 承载,按钮宽度按内容收缩、按钮文字不换行(验收里「回滚到 1.0.0」被挤成三行,就是因为行内没有换行位置,按钮被压窄)。

- [ ] **Step 1: 写失败测试** `app_row_buttons_are_in_a_wrapping_row_not_the_title_row`:在该文件 `#[cfg(test)]` 里构造一个有 `previous_version` 的 `AppSummary`(`Running`),调用抽出的纯函数 `fn row_actions(state, app, locale) -> Vec<RowAction>`(把"此刻该显示哪些按钮"从 `app_row` 里抽出来:`Stop`/`Rollback(to)`/`Logs`/`Uninstall`),断言顺序与可见性规则(`Running|Starting` 才有 Stop;有 `previous_version` 才有 Rollback;`is_transient()` 时没有 Uninstall)。这是纯逻辑测试,保证重构不丢按钮。
- [ ] **Step 2: 运行确认失败**:`cargo test -p dozer-app settings_apps_view::tests::app_row` → FAIL。
- [ ] **Step 3: 实现**:抽出 `RowAction` 与 `row_actions`;`app_row` 按"两层"组装,按钮组用 `.wrap()`。保留现有的"回滚确认"行内二次确认逻辑与日志查看器(`log_viewer`)位置不变。
- [ ] **Step 4: 运行**:`cargo test -p dozer-app settings_apps_view` → PASS。
- [ ] **Step 5: 目视验证**:隔离 GUI 里装 `py-crashy`(长失败原因)、`py-ver`(有回滚)、`py-ws-debug`;设置窗口缩到最窄宽度,截图确认每行所有按钮完整可见;点「卸载」可点中(用户来点)。
- [ ] **Step 6: Commit**:`git commit -am "fix(settings-apps): wrap the action buttons onto their own row so none is clipped"`

---

## Task 8(dozer): 升到 bytehost `v0.4.5` 并回归

**Files:** Modify: 各 `crates/*/Cargo.toml` 里 `bytehost-*` 依赖的 `tag`、`Cargo.lock`、`CLAUDE.md`(bytehost 一行的 tag)

- [ ] **Step 1: 审阅改动面**:`git -C ../bytehost log --oneline v0.4.1..v0.4.5 -- crates/bytehost-apps crates/bytehost-client crates/bytehost-webview crates/bytehost-panel`,逐条确认没有 wire 类型的**修改**(只追加)。
- [ ] **Step 2: 改 tag**:`grep -rn 'tag = "v0.4.1"' crates/*/Cargo.toml`,逐个改成 `v0.4.5`;`cargo update -p bytehost-apps`(其余 `bytehost-*` 同理)。`v0.4.5` 必须已在远程,否则解析失败。
- [ ] **Step 3: 全量验证**:`cargo build && cargo test -p dozer-app -p dozerd -p dozer-client && cargo clippy --all-targets` → 通过。
- [ ] **Step 4: 真 GUI 回归 WebSocket**:隔离环境装 `py-notes`,页面里 `ws:` 回显行出现 `ws: hello`(验收清单 2.2)。同环境装修好的 `bilingual-live`,页面直接显示问候语(6.1)。
- [ ] **Step 5: 更新 `CLAUDE.md`** bytehost 那一行的 tag;`git checkout Cargo.lock` 之前确认 lock 里 `bytehost-*` 已指向 `v0.4.5`(这次 lock 的变化是**需要提交**的,与本地 `[patch]` 带来的 `source =` 行变化区分开)。
- [ ] **Step 6: Commit**:`git commit -m "chore(deps): bytehost v0.4.1 → v0.4.5 (gateway ws fix, sample CSP fix, provenance label)"`。digger 的同步另开任务,不在本计划内。

---

## Task 9(dozer): 决策闸门 —— dozerd 断线提示(**先裁决,裁决前不写代码**)

**背景:** 验收 3.2:运行中 `kill -9` dozerd,界面 25 秒内没有任何"不可用"提示(报告预期 2 秒内显示)。机制:订阅断开只回退轮询并退避重订阅(`host_adapters/app_host.rs` 的 `SubscriptionLost`),`App.daemon_unavailable` 只有三处写入(启动连不上、设置里停止、打开项目成功即清除)。

**需要用户二选一:**

- **A. 补断线提示(推荐)。** 订阅失败达到连续 N 次(建议 2 次,约 4–6 秒)时发一次 `push_toast_keyed(LOG, Warn, …, key="app-host-subscription")`:"dozerd 连接中断,正在重连";重订阅成功时用同一个 key 推 `Info` "已恢复"。**不**写 `daemon_unavailable`(那是持久状态,有自己的三处写入约定)。测试:`SubscriptionLost` 状态机在连续失败时产出且只产出一次 `Notice` 副作用、恢复时产出恢复提示(状态机在 `bytehost-panel/src/host.rs`,`PanelEffect::Notice` 已存在,按现有测试风格加);`host_adapters/app_host.rs` 已把 `Notice` 转 Toast,无需改。
- **B. 改预期。** 把 `…a6f-acceptance-report.md` 与验收清单 3.2 的"2 秒内显示 dozerd 不可用"改成"静默退避重订阅,日志有 WARN",并在 `unfinished` 里记为有意行为。

- [ ] **Step 1:** 向用户给出 A/B,记录裁决到本计划末尾的"决策记录"。
- [ ] **Step 2:** 选 A → 另写一份小计划(涉及 `bytehost-panel` 状态机,要再发一次 tag 并 bump);选 B → 只改两处文档,提交。

---

## Task 10(dozer): 回写验收结果与剩余状态

**Files:** Modify: `docs/superpowers/manual_tests/2026-10-16-bytehost-manual-acceptance.md`、`docs/superpowers/unfinished/2026-10-16-bytehost-remaining.md`

- [ ] **Step 1:** 在手测文档里把已修复的发现行标"已修复 + 提交号",并把 2.2 的 WebSocket 结果更新为回归后的实测;`bilingual-live`/`partial-initial` 的 6.1/6.2 改用仓库样例复测并更新。
- [ ] **Step 2:** 在 `unfinished` 的 §1 里写明手工验收的真实进度(本次验收的完成项与未做项),修正"没有执行任何一项"的旧说法。
- [ ] **Step 3: Commit**:`git commit -am "docs: record acceptance fixes and refresh remaining-work status"`。

---

## Self-Review

- **Spec 覆盖:** 验收文档里的 6 项缺陷各有任务(WebSocket→T1,样例 CSP→T2,来源标签→T3,3.5 不可观测→T4,日志配色→T6,按钮裁切→T7),断线提示→T9(待裁决),发版与升级→T5/T8,文档回写→T10。观察类条目在"范围说明"里逐项说明不做的理由。
- **类型一致:** `close_after`(T1)、`log_text_color`(T6)、`RowAction`/`row_actions`(T7)在各自任务内定义并使用;T8 只引用 tag,不引用前面任务的函数。
- **根因证据:** T1 的修法建立在 hyper 源码(`conn.rs:708-720`、`role.rs:848`)上,不是推断;T1 Step 3 的"先失败再通过"会再用测试验证一遍。
- **风险点:** T1 去掉 `keep_alive(false)` 后,"空闲连接不堆积"由两样东西保证:每个非 101 响应显式 `Connection: close`、`header_read_timeout` 仍对首个请求生效;Review Focus 第 1、2 条的测试就是为此。T8 跨了 `v0.4.1 → v0.4.5` 四个版本,Step 1 的审阅是必须的。

## 决策记录

(待填:T9 的裁决、各任务的提交号。)
