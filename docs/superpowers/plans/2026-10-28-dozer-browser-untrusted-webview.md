# Dozer 浏览器面板改用不可信内容 webview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Dozer 的 Web 面板承载任意网页时,页面不再拿到任何宿主能力(`window.ipc`、`dozer://` 协议、注入脚本),与 Digger 浏览器面板同一套安全模型。

**Architecture:** 把 Dozer 浏览器面板的网页子视图从 `runtime.rs` 里和可信预览共用的构建器,换成 bytehost `bytehost_desktop::web_child::WebPool`(v0.4.16,Digger 已在用;不可信内容:无自定义协议、无注入脚本、无 IPC、导航与新窗口走白名单、下载拒绝)。状态机与视图仍是共享库 `dozer-panel-browser`,不动。Dozer 侧新增一个薄的 `browser_native` 适配层(仿 `digger/crates/desktop/src/adapters/browser_native.rs`),删掉浏览器专用的 IPC 分支。

**Tech Stack:** Rust、iced 0.14、wry 0.55(经 bytehost)、`dozer-panel-browser`、`bytehost-desktop` v0.4.16。

**Spec:** 没有单独规格。依据:`digger/docs/adr/ADR-016-browser-panel-untrusted-content.md`(安全模型与取舍)、`docs/superpowers/plans/2026-10-28-digger-browser-panel.md` 的"范围外"一节(本计划就是它建议另立的那份)。

## 现状(已读代码确认的事实)

`crates/dozer-app/src/runtime.rs` 里,浏览器面板的网页(`report_title = true`)与可信预览**共用同一个 `WebViewBuilder` 分支**(约 520–730 行),只是多挂两个导航回报钩子。因此浏览器里的**任意网页**同时拥有:

1. **`window.ipc`**:构建器装了 `with_ipc_handler`,处理 `classify_ipc` 的全部通用消息(`focus`、`mouseup`、`zoom_*`、`find_page`、`find_native`、`paste_request`、`title:<id>:<文本>`……)。
2. **注入脚本**:`preview_host_script`(焦点 / 拖拽 / 缩放 / ⌘F 三件套)被注入每个页面,另加标题回报脚本。
3. **`dozer://` 自定义协议**:`with_custom_protocol("dozer", …)` 对所有页面可用,应答 `assets::handle_protocol`:预览宿主的七个命名空间(含"已允许文件"表)、`review-trace/` 的数据端点(回显当前审阅快照)、`usage-content` / `codehealth-content` 等内嵌资源。
4. **没有导航白名单**:只有应用 webview 有 `with_navigation_handler`(约 290 行),浏览器 webview 没有;下载处理器也不在这个分支里(只在应用 webview 的构建里显式关闭)。

**尚未实测(不要当成已证实的漏洞):** 页面能否真的把自己导航到 `dozer://…` 并以该来源读到"已允许文件"或审阅快照;`paste_request` 目前在 `update.rs` 里是空操作(`Message::WebViewPasteRequest(_) => {}`),⌘V 的剪贴板回灌走 `window_events.rs` 的键盘路径,不是页面可触发的;页面发 `title:` 消息能把任意文本塞进页签标题(已知、低风险)。**Task 0 先在隔离环境里复现,再决定紧迫程度。**

## Global Constraints

- 不得动 Dozer 里**预览 / 编辑器 / 内容面板**的构建路径:它们是可信内容,继续用现有构建器与 `dozer://`。只换浏览器面板。
- 共享库 `dozer-panel-browser` 的接口不改(Digger 也在用);若必须改,走 bytehost → Dozer libs tag → Digger 重钉的发版链路,每次推送先问用户。
- 重钉 / 发版时 Dozer 要临时挪开 `.cargo/config.toml` 才能得到干净的 `Cargo.lock`。
- 门禁:全 workspace 测试、`check-i18n`、`check-log-scope`、`check-shared-deps`、fmt、clippy(`dozer-hook/src/opencode_install.rs:106` 的既有 `needless_borrow` 报错与本计划无关)。
- 日志统一走 `dozer_core::log_*!`,面板日志来源名是 `web`(`PanelKind` 对应的名字)。
- 测试覆盖只算能自动化的;真实窗口的层级、焦点、输入法、几何落验收清单。

## Review Focus

1. **页面仍能到达宿主**:浏览器页面通过 `window.ipc`、`dozer://`、重定向到 `javascript:` / `dozer://` 触达宿主。→ 钉死测试(浏览器分支不得再出现 `with_custom_protocol` / `with_ipc_handler` / `with_initialization_script`),Task 0 的复现页换成回归页。
2. **丢功能被当成回归**:浏览器里的 ⌘F 页内查找、页面里的 ⌘V 回灌、页面缩放热键、页签拖进 webview 后松手结束拖拽,这些现在都靠注入脚本 + IPC 实现。→ Task 2 逐项给出处置(保留 / 换实现 / 明确放弃),不得悄悄丢。
3. **页签拖拽卡死**:拖拽中鼠标进入 webview,winit 收不到 `Released`,现在靠页面里的 `mouseup` IPC 结束拖拽(`WebViewMouseUp`)。没有脚本后这条路断了。→ Task 2 的 `drag_ending_over_the_webview_still_ends`。
4. **切项目 / 关页签 / 最后一个页签**:池与状态不一致。→ 复用 Digger 的 `prepare` 状态机测试形状(一个项目一代,换项目整池重建)。
5. **键盘**:没有注入脚本后 ⌘C/⌘V/⌘X/⌘A 要靠应用菜单里的 Edit 菜单,Dozer 自己有没有主菜单、会不会与已有快捷键处理冲突。→ Task 1 先查清再定,验收清单保留回归项。

---

## File Structure

- Create: `crates/dozer-app/src/browser_native.rs` — 几何、`prepare` 状态机、事件翻译(仿 Digger)。
- Modify: `crates/dozer-app/src/runtime.rs` — 删浏览器分支(`report_title`、标题脚本、两个导航回报钩子),保留可信预览分支。
- Modify: `crates/dozer-app/src/extensions/browser.rs`、`app/update.rs`、`platform/window_events.rs` — 路由 `Message::Browser*` 到新适配层;处理上面"丢功能"的去留。
- Modify: `crates/dozer-app/Cargo.toml` — 已依赖 `bytehost-desktop`;确认带上 `web_child` 所需的 objc2 依赖随它传递。
- Test: 新增测试在各自模块;钉死测试读生产源码。

## Tasks

### Task 0: 复现暴露面(先于任何改动,隔离环境)

**Files:** Create: `spike/dozer-browser-exposure/`(自成工作区,不进 CI;或复用 `spike/browser-probe`)。

**Interfaces:** Produces: 一份结论 `docs/superpowers/reviews/2026-10-28-dozer-browser-exposure.md`,逐条写"能 / 不能 / 未测"。

- [ ] **Step 1: 造复现页** `exposure.html`(放 `/tmp/dozer-exposure/`,经 `file://` 打开),按钮分别:读 `window.ipc` 是否存在;`postMessage('title:1:AAAA')` 看页签标题是否被改;`location='dozer://review-trace/data.json'` 与 `dozer://flyfish/…`(命名空间名以 `dozer_panel_preview::assets` 为准)看是否跳转成功;`fetch('dozer://…')` 看是否被 CORS 挡;`<img src="dozer://…">` 看是否能加载;`location='javascript:…'`;`<a download>`。每个结果写进页面里的日志区。
- [ ] **Step 2: 在隔离的 Dozer 里跑**(`HOME` 与数据目录指向临时目录,不碰用户的 Dozer 数据和 daemon;**不要**在用户正在运行的 Dozer 里做)。用 Orca 截图读日志区(`orca computer get-app-state`;注意 Orca 的合成点击进不了 iced 控件,按钮要靠页面自己的 `setTimeout` 自动执行,不要指望点击)。
- [ ] **Step 3: 写结论文档**,明确每条是"已证实可达 / 已证实不可达 / 没测成"。**若有"已证实可达读取文件或审阅快照",立即告知用户并把本计划提到最高优先级。**
- [ ] **Step 4: 提交**(结论文档 + spike)。

### Task 1: 评估 Dozer 的键盘与菜单现状,定 Edit 菜单策略

**Files:** Read: `crates/dozer-app/src/platform/window_events.rs`(⌘C/⌘V/⌘F 处理)、主菜单相关代码。

- [ ] **Step 1: 查清** Dozer 是否已有主菜单(`NSMenu` / `setMainMenu`);浏览器 webview 的 ⌘C/⌘V/⌘X/⌘A 现在靠什么生效(注入脚本的 `focus` IPC 让 WKWebView 成为 first responder,再配合 `window_events.rs` 的键盘处理)。
- [ ] **Step 2: 写结论到本计划 Task 1 下**,并定方案:装 `install_standard_edit_menu()`(与 Digger 同)还是沿用 Dozer 自己的键盘处理。若 Dozer 已有 Edit 菜单,不要重复装。**这一步的结论可能要问用户。**

### Task 2: 逐项处置"靠注入脚本实现的浏览器功能"

**Files:** Modify: `runtime.rs`、`app/update.rs`、`platform/window_events.rs`。

- [ ] **Step 1: 列出并给出去留表**(写进本计划),至少含:

| 功能 | 现在的实现 | 去掉脚本后 | 待定 |
|---|---|---|---|
| ⌘F 页内查找(`find_page` / `find_native`) | 页面脚本 → IPC → 宿主调 `findString:` | 页面不再能发消息;宿主可在**键盘事件**层响应 ⌘F 直接对激活视图调原生查找 | 保留 / 放弃(Digger v1 放弃了) |
| ⌘V 回灌(`paste_request`) | 键盘路径读剪贴板 + `evaluate_script` 回灌 | Edit 菜单的 Paste 直达页面 | 放弃自定义回灌 |
| ⌘/Ctrl +/- 缩放 | 页面脚本 → `zoom_*` IPC | 宿主在键盘层处理,池按全局 scale 缩放(`WebPool::sync` 已带 `scale`) | 确认 |
| 页签拖进 webview 后松手(`WebViewMouseUp`) | 页面 `mouseup` IPC | 需要替代:窗口级鼠标事件 / 拖拽超时 | **新设计,有风险** |
| 标题 / 页内跳转回报 | 脚本 + `on_page_load` | `WebPool` 的原生回调,`WebEvent::{TitleChanged, PageLoaded}` | 直接替换 |

- [ ] **Step 2: 失败测试** `drag_ending_over_the_webview_still_ends`:模拟拖拽中指针进入浏览器区域后松手,拖拽状态必须结束(Review Focus 3)。
- [ ] **Step 3: 实现替代**,并提交每项去留的说明。

### Task 3: `browser_native.rs` 与切换

**Files:** Create: `crates/dozer-app/src/browser_native.rs`;Modify: `runtime.rs`、`extensions/browser.rs`、`app/state.rs`(池所有权与事件通道)。

**Interfaces:** Consumes: `bytehost_desktop::web_child::{WebPool, WebTabSpec, WebEvent, WebNav}`;库 `State::desired_webviews()`、`chrome_top_px()`。Produces: `BrowserNative { prepare, sync, nav, poll_stalled, update }`,与 Digger 同形。

- [ ] **Step 1: 失败测试(几何)** `webview_rect_uses_chrome_top_px_and_the_existing_web_geometry`:与 `webview_geometry.rs` 的 `PanelKind::Web` 分支(`y = y0 + browser_chrome_top_px()`、`h = avail_h − chrome − 8`、左右各 8)逐项相等,缩放 1 与 2;源码钉死无手写像素常数。
- [ ] **Step 2: 失败测试(生命周期)** `project_switch_destroys_the_previous_projects_pages`、`hidden_panel_parks_and_does_not_destroy`、`stale_generation_events_are_ignored`、`closing_the_last_tab_keeps_one_blank_view`(形状同 Digger `browser_native.rs` 的测试)。
- [ ] **Step 3: 失败测试(钉死)** `browser_webview_is_untrusted`:读 `runtime.rs` 与 `browser_native.rs` 生产源码,浏览器路径**不得**出现 `with_custom_protocol`、`with_ipc_handler`、`with_initialization_script`;全 Dozer 里 `web_child` 之外不得有为浏览器面板建 wry 视图的调用。
- [ ] **Step 4: 实现并接线;删除** `runtime.rs` 的 `report_title` 分支、`BrowserTitle` / `BrowserNavigated` / `BrowserNewWindow` 消息(由 `WebEvent` 取代)。首页的全局浏览器(`project_id = None`)也走同一路径。
- [ ] **Step 5: 变异验证:** 把浏览器路径改回装 IPC / 协议、去掉停放、去掉代号校验,各自让对应测试变红。
- [ ] **Step 6: 门禁** 全部通过后提交。

### Task 4: 验收与收口

- [ ] 在 Dozer 验收清单(新建 `docs/superpowers/reviews/` 下一份)加"浏览器"一节:Task 0 的复现页在改动后**全部不可达**;⌘A/⌘C/⌘V/⌘X 在页面里、地址栏、其它输入框里都正常;页签拖拽进出 webview 不卡死;首页全局浏览器与项目内浏览器;收藏夹;`file://`。
- [ ] 记忆更新;**请用户目测验收后停下。**

## 范围外

- 预览 / 编辑器 / 内容面板的 webview(可信内容)。
- 把 Dozer 的应用 webview(`build_app_webview`)并过来:它已经是受限路径。
- ⌘F、下载管理、无痕模式等新功能。

## 需要用户拍板的点

1. **⌘F 页内查找:** 保留(宿主在键盘层调原生查找)还是像 Digger v1 一样放弃?
2. **Task 0 的结论若证实可读文件 / 审阅快照:** 是否先发一个只拦 `dozer://` 导航的快速止血补丁,再做完整迁移?
3. **Edit 菜单:** Task 1 查清后若 Dozer 已有自己的键盘处理,是沿用还是统一成 Edit 菜单。
