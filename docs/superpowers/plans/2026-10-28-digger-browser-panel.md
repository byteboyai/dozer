# Digger 浏览器面板 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 Digger 加一个浏览器面板(多页签、地址栏、后退/前进/刷新、页内跳转地址栏跟随、`target=_blank` 新开页签、收藏夹),复用共享库 `dozer-panel-browser` 的状态机与视图,**网页由独立的、按"不可信内容"设计的 wry 子视图承载**。

**Architecture:** 三层。① `dozer-panel-browser`(已有、已共享):页签/地址栏/收藏夹状态机与视图,仅加两个只读访问器。② **bytehost 新增两块**:`bytehost-webview::browse`(纯策略:导航白名单、IPC 白名单)与 `bytehost-desktop::web_child`(多页签 wry 子视图池 + 受限 builder),Dozer 不改、不迁。③ Digger 侧薄适配:`BrowserDisplay` 实现(用 byteui 页签部件)、`browser_native.rs` 驱动池、`product.rs` 接线、`shell.rs` 注册面板。收藏夹**不新建存储**:`dozer_client::Client` 已实现 `BookmarkService`,`diggerd` 的库里本来就有书签表。

**Tech Stack:** Rust workspace;iced 0.14(标准 `iced::application`,**没有** Dozer 那套每帧运行的自定义事件环);wry 0.55.1(`with_document_title_changed_handler` / `with_on_page_load_handler` / `with_new_window_req_handler` / `with_navigation_handler` / `with_download_started_handler` / `with_incognito` 都有);byteui `v0.5.0`;bytehost `v0.4.14`;Dozer libs `libs-2026-10-28d`。

**Spec:** 无独立设计稿。依据:`docs/superpowers/plans/2026-10-27-panel-browser-lib.md` 及其报告(库的边界与已知缺口);`2026-10-28-x1-cross-repo-migration.md`(byteui 页签/菜单部件、bytehost `native_child` 已就位);Digger `docs/adr/ADR-013`(markdown 宿主的 wry 设置钉死先例);`CLAUDE.md`「应用 webview 走独立 builder,不得复用预览/浏览器的构建路径」;`2026-10-27-panel-browser-lib.md`「不在本 plan 内」里"Digger 接入浏览器面板,另起一期"就是本计划。

## 已裁决与待裁决

**用户已裁决(2026-10-28):**
- **`file://` 必须支持**,地址栏不拒绝。这条直接决定下面的安全设计:能打开本地文件的页面也**不得**拿到任何宿主权限。

**我的默认(Task 5 前请用户确认,可改):**
1. 面板放**右栏、单栏(无配对)、紧随 `conversations`**;wire id `digger.browser` 一旦发布不可改。
2. 状态**按项目**(没有 Dozer 那种"首页全局浏览器");收藏夹仍分全局 / 本项目两档(服务层已支持)。
3. 切换项目时**销毁**前一个项目的页面(各页签的网址保留在状态里,切回来重新加载),与 Dozer 一致;不做"后台常驻"。原因:库里页签 id 是每个 `Tabs` 自己的计数器,不同项目会撞号,池若常驻必须按 `(项目, 页签)` 建键,v1 不值得。
4. **v1 不做**:⌘F 页内查找、页面里的 ⌘V 剪贴板回灌(待 Task 0 的 S2 结论再定)、下载、开发者工具、多窗口。
5. 数据存储**持久**(登录态保留),不用无痕。

**需要你裁决:**
- **裸路径 `/Users/x/a.html` 仍被拒**,提示"浏览器不支持打开本地文件"——和"`file://` 要支持"矛盾。Digger 是新面板,建议**自动补成 `file://`**(库侧改一处,Dozer 同步变化);或保持现状。**建议补。**

## Global Constraints

- **不可信内容原则:** 浏览器子视图**不注册任何自定义协议**(没有 `dozer://`、没有 `digger-*://`)、**不注入脚本**、**不装 IPC 处理器、不注入任何初始化脚本**(S2 实测:Edit 菜单足以让 ⌘C/⌘V/⌘X 生效,中文输入法本来就正常),标题与页面加载**走 wry 原生回调**。禁止复用 `build_app_child` 或 Digger 现有可信 `native_child` 的 builder。
- **wry 默认值不是安全默认**:下载、开发者工具、媒体/定位权限、链接预览、新窗口、导航,每一项必须在 builder 里显式写,并有**读生产源码的钉死测试 + 变异验证**(改一项值,测试必须变红)。
- 导航白名单:`http`、`https`、`file`、`about:blank`;其余(`javascript:`、`data:`、`blob:`、`view-source:`、`dozer:`、自定义 scheme、`tel:`、`mailto:`…)一律拒绝,**新窗口请求走同一白名单**才新开页签。
- 共享库发版联动沿用 X1 的教训:bytehost 先发,Dozer 重钉并发 libs tag,Digger 同日重钉;byteui/bytehost/bytegit 钉版两边一致(`check-shared-deps.sh` 0 阻断);**每次推送、打 tag 前先问用户**。
- Dozer 的外观与行为**不变**(本计划不碰 Dozer 的浏览器 webview 路径与 `runtime.rs`)。`dozer-panel-browser` 只加只读访问器,不改既有公开项。
- Digger 约束:不碰工作区里 13 份已删除的 S6 文档;**Digger 没有远端,提交只在本地**;提交带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`;提交前 `git diff --cached --stat`;**检查类命令单独跑、单独读退出码,通过后再提交**(不要用分号串联);**不在用户主工作区用 `git stash`**。
- 门禁一律在干净快照跑;面板 UI 字体用系统默认;面板日志来源名用 `browser`(Digger 无 `check-log-scope`,沿用现有 adapters 的日志写法)。
- 测试覆盖只算"能自动化的";webview 真实渲染、焦点、输入法、层级无头测不了,**全部落到验收清单 I 区**(见 Task 10)。

## Review Focus

1. **页面拿到宿主权限**:`file://` 页面或恶意站点调用 `window.ipc`、访问自定义协议、通过 `window.open`/重定向跳到 `dozer://`/`javascript:`。→ Task 1 `browse_navigation_*`(逐个 scheme 的允许/拒绝表)、Task 2 `web_child_pins_untrusted_content_settings`(无 `with_custom_protocol`、默认拒绝媒体权限等)。
2. **媒体/定位权限被静默批准**:S3 已证实 wry 对摄像头/麦克风请求**无条件 `Grant`**,目前只因 `navigator.mediaDevices` 没暴露而够不着(休眠的隐患,依赖 wry/WebKit 默认值)。定位未测。→ Task 2 `media_capture_requests_are_denied_by_the_proxy_delegate`(代理拒绝,纵深防御)。
3. **webview 与 iced 层级**:webview 恒在 iced 之上。iced 浮层(收藏星菜单、页签溢出菜单、输入框右键)打开时,若 webview 仍可见会盖住菜单;切到别的面板后 webview 残留成"幽灵"。→ Task 7 `native_menus_do_not_need_hiding_but_iced_overlays_hide_the_webview`、`leaving_the_panel_destroys_or_parks_every_webview`,Task 2 的 `Probe` 读真实 NSView 状态。
4. **页签 id 跨项目撞号与池生命周期**:切项目、关项目、关最后一个页签(库回落到 `about:blank`)时池与状态不一致。→ Task 7 `project_switch_destroys_the_previous_projects_webviews`、`closing_the_last_tab_keeps_one_blank_webview`。
5. **地址栏在标准 iced 下的焦点与全选**:Dozer 靠每帧 `interface.operate` 读焦点;Digger 没有这个循环。焦点判断滞后会让"全选一次性位"重复触发或丢失,中文输入法候选被打断。→ Task 6 `addr_focus_bridge_*`;输入法本身落验收清单 I 区。

---

## 文件结构

| 位置 | 动作 | 职责 |
|------|------|------|
| `bytehost/crates/bytehost-webview/src/browse.rs`(新) | 纯策略 | `allows_browse_navigation(url)`,无 wry/iced |
| `bytehost/crates/bytehost-desktop/src/web_child.rs`(新) | 执行器 | 多页签子视图池、受限 builder、事件回报、`Probe` |
| `dozer/crates/dozer-panel-browser/src/state.rs` | 加只读访问器 | `State::addr_text() -> &str`(仅此一项,除非 Task 0 的 S4 证明还需要别的) |
| `digger/crates/desktop/src/shell.rs` | 注册 | `browser_id()`、meta、rules、默认布局、旧布局迁移 |
| `digger/crates/desktop/src/adapters/browser.rs`(新) | 面板 | 按项目的 `State`、服务、`update` 路由、焦点桥、地址栏菜单 |
| `digger/crates/desktop/src/adapters/browser_display.rs`(新) | 展示 | `BrowserDisplay` 实现(byteui `panel_tabs` + 分隔条) |
| `digger/crates/desktop/src/adapters/browser_native.rs`(新) | 原生层 | 驱动 `web_child`:几何、可见性、事件→库 `Message` |
| `digger/crates/desktop/src/product.rs` | 接线 | `Msg` 变体、激活/可见、几何、订阅、关项目清理 |
| `digger/crates/desktop/examples/browser_gallery.rs`(新) | 示例 | 仿现有 gallery,人工看视图 |
| `digger/docs/adr/ADR-016-browser-panel-security.md`(新;当前最新是 ADR-015) | 文档 | 不可信内容的 wry 设置与理由 |

---

### Task 0: 技术探针 —— 全部完成(2026-10-28;S5 沿用既有定论未做)

六个会推翻设计的未知项,在一个**一次性的小程序**里实测(`dozer/spike/browser-probe/`,已在仓库根 `Cargo.toml` 的 `exclude` 里,随时可删),结论与原始输出在 `digger/docs/superpowers/reviews/2026-10-28-browser-panel-spikes.md`。**结论速览:**

- **S1 ✅ `file://` 直接能用**,不需要 `loadFileURL`;同目录子资源能加载;页面 `fetch` 不到其它本地文件。含空格/中文的 URL 在 macOS 26 上被自动百分号编码、不 panic(旧系统未测)。**发现:wry 没有"导航失败"回调。**
- **S2 ✅ 装一个标准 Edit 菜单即可,不需要注入任何脚本。** 打字、中文输入法本来就正常;没有 Edit 菜单时 ⌘C/⌘V/⌘X 完全不生效(`copy`/`paste`/`cut` 事件 0 次),装了就生效(2/4/2 次)。**用户已确认:** 装了 Edit 菜单后 webview 里 ⌘A 能全选,iced 输入框的 ⌘C/⌘V/⌘X/⌘A 不受影响;不装的话 webview 里这些快捷键都不能用。
- **S3 ✅ wry 对所有来源的摄像头/麦克风请求无条件 `Grant`,但当前 WKWebView 里 `navigator.mediaDevices` 是 `undefined`,够不着(休眠的隐患)**;用 objc2 代理委托拒绝并转发其余方法**已验证可行**。
- **S4 ✅ 标准 iced 下 `focus`/`is_focused` 可用。**
- **S5 ➖ 未做**(项目既有定论)。
- **S6 ✅ 创建约 33ms/页签;8 页签 WebContent 约 +74MB;销毁不会立刻释放内存。**


- [ ] **S1 `file://` 能否显示:** wry 0.55 子视图 `load_url("file:///…/a.html")` 在 WKWebView 里是否真的渲染(WKWebView 对 `file://` 常需要 `loadFileURL:allowingReadAccessTo:`);含空格、中文路径;同目录相对资源(`<img src>`、`<link>`)是否能加载;`file://` 页面能否 `fetch` 其它 `file://`。**若普通 `load_url` 渲染不了,记下需要的 wry/objc2 调用,计划 Task 2 据此加分支。**
- [ ] **S2 子视图里的键盘:** 在 Digger 的 winit 窗口上,点进页面输入框后:能否打字、⌘C/⌘V/⌘A/⌘X 是否直达页面、中文输入法候选是否正常、焦点是否需要 Dozer 那样的"JS mousedown → IPC → `webview.focus()`"桥。**决定 Task 2 要不要注入 init 脚本。**
- [ ] **S3 未信任页面的权限默认值:** 打开一个请求摄像头/麦克风/定位的测试页:wry 0.55 是静默批准、弹系统提示还是拒绝?能否显式改成拒绝(`WebViewBuilderExtDarwin` 有无对应项,或需 objc2 实现 `WKUIDelegate`)?
- [ ] **S4 标准 iced 下的焦点读取:** `iced::widget::operation::is_focused(addr_field_id())` 作为 `Task<bool>`,在鼠标点击 / 键盘 Tab / 失焦后触发,能否在一帧内拿到正确值;全选一次性位用 `operation::select_all` 是否稳定。
- [ ] **S5 层级与隐藏:** 子视图可见时,Digger 现有的原生菜单(`byteui::native_menu`)是否天然盖在它上面;iced 自绘浮层(`menu_overlay`)是否被盖;隐藏(`set_visible(false)` / 移出屏)后 WKWebView 是否继续消耗 CPU。
- [ ] **S6 池成本:** 同时存活 8 个页签的内存、创建/销毁耗时;隐藏页签是否被系统挂起。

提交:报告 + `spike/` 小程序(**不进 workspace 默认成员**,避免拖慢 CI)。

### Task 1: `bytehost-webview::browse`(纯策略,先写测试)—— 已完成(2026-10-28,bytehost `feat/browse-policy` `b245b92`)

**结果:** `allows_browse_navigation` 与 `allows_new_tab`(同表)已实现,6 条测试;bytehost 全部 440 通过/0 失败,clippy/fmt/`check-deps.sh` 通过。变异验证:去掉控制字符/反斜杠预检、`file` 放行任意主机、去掉首尾空白检查、`about` 放行任意、兜底分支放行,各让对应测试变红(一个"放行 javascript"的变异本身无效——`javascript:` 无主机,后面的主机检查仍会拒绝——已换成有意义的兜底变异)。**实现发现:** 标准 `url` 解析器会**悄悄剥掉**制表符/换行、把反斜杠当斜杠、trim 首尾空白、把 `http:///x` 规范成 `http://x/`,所以必须先拒绝前三类再解析,最后一类按解析器的结果放行(WebKit 同样处理)。下面的勾选项保留作记录。

**Files:** Create `bytehost/crates/bytehost-webview/src/browse.rs`;Modify `lib.rs`。

**Interfaces:**
- Produces:`pub fn allows_browse_navigation(url: &str) -> bool`;(S2 实测后**不再需要 IPC 白名单**:浏览器子视图不装 IPC,所以这里只有导航策略这一个函数。)

- [ ] **Step 1: 失败测试** `browse_navigation_allows_only_the_listed_schemes`:允许表 `https://a.com`、`http://a.com:8080/x?y#z`、`file:///Users/me/My%20File.html`、`file:///Users/me/中文.html`、`about:blank`;拒绝表 `javascript:alert(1)`、`data:text/html,x`、`blob:https://a.com/x`、`view-source:https://a.com`、`dozer://a/b`、`digger-review://review-trace/x`、`tel:123`、`mailto:a@b.c`、`ftp://a`、空串、含控制字符/反斜杠的 URL(`https://a.com/\n`、`https://a.com\\@evil/`)、大小写 `HTTPS://A.COM`(**允许**,scheme 不区分大小写)。
- [ ] **Step 2: 确认失败。**
- [ ] **Step 3: 实现。** 一行方法说明:用 `url` crate 解析太重,按 `strip_prefix` + 小写 scheme 比对,和 `AppOrigin::allows_navigation` 同风格(拒绝控制字符与反斜杠)。
- [ ] **Step 4: 通过 + clippy/fmt。Step 5: 提交。**

### Task 2: `bytehost-desktop::web_child`(多页签池 + 受限 builder)—— 已完成(2026-10-28,bytehost `feat/browse-policy` `54acfec`,未推送)

**结果:** `web_child/{plan,edit_menu,apple,pool}.rs`;bytehost 全 workspace(`--all-features`)874 通过/0 失败,clippy `-D warnings`/fmt/`check-deps.sh` 通过(裸 `cargo clippy --all-targets` 会因 `bytehost-client` 的 `embed` 示例缺 feature 失败,与本改动无关,要带 `--all-features`)。真实窗口冒烟(`pool-probe`):Edit 菜单只装一次;`javascript:` 页签被拦成 `about:blank`;file 页面标题/加载事件正常;切换/刷新正常;换 generation 销毁旧页签;`destroy_all` 句柄归零;不可达地址产出 `NavigationStalled`。**与上面形状的偏差:** ① `plan_pool` 没有 `active` 参数(可见性由每个 `WebTabSpec.visible` 表达);② `encode_for_wry` 放在 `bytehost-webview::browse`(`5daf8d4`),`web_child::wry_target` 把它和白名单合成一步;③ 新增 `NavTracker`、`LoadVerdict`、`WebEvent::NavigationBlocked`。**冒烟抓到的真 bug:** 不可达地址只会触发初始空白文档的 `about:blank` Started/Finished,会清掉停滞计时、还会把页签网址悄悄改成 `about:blank`;修为 `NavTracker::on_page_load`(有待定导航时忽略 `about:blank`)。下面的勾选项保留作记录。

**Files:** Create `bytehost/crates/bytehost-desktop/src/web_child.rs`;Modify `lib.rs`。

**Interfaces:**
- Consumes:Task 1 的 `allows_browse_navigation`。
- Produces(形状,以 `native_child` 的 `Desired`/`plan`/`Host` 为参照,但**按页签池而非单槽**):`WebTabSpec { id: usize, url: String, visible: bool }`;`pub fn plan_pool(existing: &[(usize, String)], desired: &[WebTabSpec], active: Option<usize>) -> Vec<PoolAction>`(纯函数:`Create/Navigate/Show/Hide/Destroy`);`pub struct WebPool`(`new(epoch)`、`sync(window, generation, desired, rect, scale) -> Task<E>`、`nav(window, id, WebNav::{Back,Forward,Refresh})`、`probe(..)`);事件 `WebEvent::{PageLoaded{id,url}, TitleChanged{id,title}, NewTabRequested{url}, NavigationStalled{id,url}}`。

- [ ] **Step 1: 失败测试(纯函数)** `plan_pool_*`:空→创建;URL 变→只 `Navigate`;只有激活页签可见;页签消失→`Destroy`;关最后一个页签后库回落的 `about:blank` 仍有一个视图;项目切换(换 `generation`)→全部 `Destroy` 再 `Create`。
- [ ] **Step 2: 失败测试(钉死,读生产源码)** `web_child_pins_untrusted_content_settings`:**必须出现** `.with_devtools(false)`、`.with_download_started_handler(|_, _| false)`、`.with_navigation_handler(`(且内部调 `allows_browse_navigation`)、`.with_new_window_req_handler(`(且同走白名单)、`.with_document_title_changed_handler(`、`.with_on_page_load_handler(`、`.with_allow_link_preview(false)`,以及 S3 结论里的权限拒绝写法;**不得出现** `with_custom_protocol`、`with_initialization_script`、`with_ipc_handler`、`NewWindowResponse::Allow`、`.with_devtools(true)`、`.with_incognito(true)`(v1 要持久)。
- [ ] **Step 3: 确认失败 → 实现。** 池的句柄表是模块私有 `thread_local`,**与 `native_child`、`app_webview` 的永不共用**;所有 wry 调用在 `iced::window::run` 里。**`file://` 不需要特殊加载路径**(S1:普通 `with_url`/`load_url` 即可)。
- [ ] **Step 3b(探针带来的新增项,先写失败测试):**
  - `urls_are_percent_encoded_before_reaching_wry`:纯函数 `encode_for_wry(url) -> String`,含空格 / 中文 / 已编码 / 非法字符的 `http`、`file` URL 都得到合法 URL,**幂等**(编码两次结果不变),不改动已编码部分(S1:macOS 26 上 wry 自己会编码、不 panic,但旧系统可能返回 nil 然后 `unwrap` panic,所以宿主自己编码,并在 ADR 写明最低 macOS)。
  - `a_navigation_that_never_starts_is_reported_as_failed`:wry 没有"导航失败"回调(S1:不可达地址不会有 `Started`/`Finished`);池在发出导航后 N 秒内没收到该页签的 `Started` 就产出 `WebEvent::NavigationStalled{id,url}`,宿主据此提示"无法打开"。
  - `media_capture_requests_are_denied_by_the_proxy_delegate`:把 S3 的 `DenyMediaProxy` 搬进来(依赖 `objc2`、`objc2-web-kit` 的 `WKUIDelegate`/`WKFrameInfo`/`WKSecurityOrigin`);断言:装上后 `UIDelegate` 的类是代理、对新窗口 / 文件选择 / 媒体捕获三个可选方法 `respondsToSelector` 仍为真、直接调用媒体捕获给 `Deny`;代理被 `thread_local` 保活(WKWebView 对委托是 weak 引用);源码钉死测试里加上"必须调用代理安装函数"。
  - **S2 已决(实测)**:Edit 菜单让 ⌘C/⌘V/⌘X 直达页面 → **`web_child` 不注入任何初始化脚本、不装 IPC 处理器**,页面看不到 `window.ipc`。钉死测试里 `with_initialization_script` 与 `with_ipc_handler` 都进"不得出现"。因此 Task 1 的 `BrowseIpc`(nonce 白名单)**不再需要,删除**。
  - `standard_edit_menu_items() -> Vec<EditItem>`(纯函数:Cut `cut:` ⌘X、Copy `copy:` ⌘C、Paste `paste:` ⌘V、Select All `selectAll:` ⌘A,目标为空)与 `install_standard_edit_menu()`(objc2,仅 macOS,必须在主线程,幂等);测试断言四项的标题、选择器、快捷键。
- [ ] **Step 4: 变异验证:** 依次把 devtools 改 `true`、下载处理改放行、导航处理改恒 `true`、删掉新窗口白名单检查、跳过代理委托安装,**每一种都必须让钉死测试变红**,再恢复(用 python 改,不要用多行 `sed`)。
- [ ] **Step 5:** `cargo test --workspace --lib --tests`、clippy、fmt、`scripts/check-deps.sh`;提交。

### Task 3: 库只读访问器 —— 已完成(2026-10-28,Dozer main)

**结果:** `State::addr_text()` 已加(`addr_text_mirrors_the_address_buffer`)。用户裁决「裸路径自动补 `file://`」已落地:`Tabs::addr_submit` 对 `/…`、`~/…` 返回 `file://` URL(`~/` 用 `$HOME` 展开;`%`、空格、`#`、`?` 转义,中文原样留给宿主编码);`//host/..` 与无 `$HOME` 的 `~/` 仍报 `error.local_path`。两条旧测试按裁决改期望(`addr_submit_rejects_local_paths_and_blank_input`→`addr_submit_opens_local_paths_and_ignores_blank_input`,另一条同理),新增 `local_paths_become_file_urls_with_reserved_characters_escaped`;验收 H13 已同步。库 56 测试通过,Dozer 全 workspace 通过(`product_isolation` 需先 `cargo build -p dozerd --example product_daemon`)。**注意:** 库 Dozer 也在用,Dozer 浏览器里同样会打开本地路径。下面的勾选项保留作记录。

**Files:** Modify `dozer/crates/dozer-panel-browser/src/state.rs`;Test `tests.rs`。

- [ ] **Step 1: 失败测试** `addr_text_mirrors_the_address_buffer`:`State` 在 `AddrInput("abc")` 后 `addr_text() == "abc"`,`AddrSubmit` 后为空。
- [ ] **Step 2: 实现** `pub fn addr_text(&self) -> &str`(转发 `Tabs::addr_buffer`)。**仅此一项**;若 Task 0 的 S4 发现还需要别的只读入口(例如当前激活页签的 URL 以便"复制网址"),在这里一并加,每个配一条测试。
- [ ] **Step 3:** 若 裁决「裸路径自动补 `file://`」:改 `Tabs::addr_submit` 对以 `/`、`~/` 开头的输入返回 `file://` URL,**更新**现有测试 `addr_submit_rejects_local_paths_and_blank_input` 与 `addr_focus_and_submit_parses_url_and_rejects_local_paths` 的期望(这是用户裁决带来的有意变化,提交说明单独写明),并在验收清单 H13 里同步。
- [ ] **Step 4:** 库全部测试 + Dozer 全 workspace 测试通过。提交。

### Task 4: 发版联动

**已完成(2026-10-28):** bytehost `v0.4.15`(`54acfec`)、Dozer `libs-2026-10-28e`(`3fb3e0da`)已推送;Digger 本地重钉提交(无远端),测试 836 通过/0 失败,fmt/clippy 干净。Dozer 全 workspace 3589 通过;全 workspace clippy 有 1 个与本次无关的既有报错(`dozer-hook/src/opencode_install.rs:106` `needless_borrow`,rust 1.98)。Digger 此时还没有 `dozer-panel-browser` 依赖,Task 5/6 再加(用同一 libs tag)。

- [ ] **Step 1:** bytehost 版本 `v0.4.14` → **`v0.4.15`**(纯追加,两个新模块),**请求用户确认**后推送并打 tag。
- [ ] **Step 2:** Dozer 重钉 bytehost、门禁(全 workspace 测试、`check-i18n`、`check-log-scope`、`check-shared-deps`、fmt),发新 libs tag,**请求确认**后推送。
- [ ] **Step 3:** Digger 同日重钉,先不改代码,现有测试须 0 失败(基线 836)。提交。

### Task 5: Digger 注册面板(`shell.rs`)

**已完成(2026-10-28,Digger 本地):** `browser_id()`/`all_panels`/`meta`(浏览器,`IconKind::Globe`)/`defaults()`(右栏 agent、群聊、会话、**浏览器**、应用、用量)/`browser_after_conversations` 迁移,无 `rules` 条目;6 条新测试 + 10 处既有"右栏完整顺序"期望更新;变异(迁移锚点改成 agent)让多项测试变红。**计划外:** `product.rs` 目录登记必须同步加(`registers_product_panels` 会红),面板数 10→11,`surfaces` 先空;此提交后浏览器面板能在图标栏看到但内容为空白,Task 8 接线前不要发版。Digger 842 通过/0 失败,fmt/clippy 干净。

**Files:** Modify `digger/crates/desktop/src/shell.rs`。

- [ ] **Step 1: 失败测试:** `browser_panel_id_is_the_persisted_wire_name`(`digger.browser`)、`the_default_layout_places_the_browser_after_conversations`、`an_old_file_without_the_browser_gets_it_after_conversations_and_keeps_selection`、`browser_migration_is_idempotent`、`an_old_file_with_an_empty_right_rail_gets_the_browser_first`、`browser_has_no_pair_rules_like_markdown`(`rules(&browser_id())` 为 `None`,与 `digger.markdown`/`digger.info`/`digger.apps` 同类:单栏、不配对,所在侧由 `defaults()` 与迁移决定)——**逐条仿群聊(`group_chat_after_agent`)的同名测试**。
- [ ] **Step 2: 实现:** `browser_id()`、`all_panels()`、`meta`(标题"浏览器",图标 `IconKind::Globe`)、`rules` **不加条目**(同 markdown,落到 `_ => None`)、`defaults()` 右栏顺序、读盘前 JSON 迁移 `browser_after_conversations`(幂等、只改读入文本不写盘、更高版本文件不覆盖)。
- [ ] **Step 3:** `shell.json` 的 wire 名**发布即冻结**,在测试里钉死。门禁 + 提交。

### Task 6: 面板适配(`adapters/browser.rs` + `browser_display.rs`)

**已完成(2026-10-28,Digger 本地 `61a56b5`):** `adapters/{browser,browser_display,browser_addr_menu}.rs` + `product.rs` 接线(`Msg::{Browser,BrowserAddrEdit,BrowserFocusFrame,BrowserAddrFocused}`、面板可见时的 16ms 探针订阅、`body()`、`on_visible` 激活、三处项目关闭钩子、收藏夹流式回传);依赖 `dozer-panel-browser`(`libs-2026-10-28e`)。Digger 854 通过/0 失败(+12),fmt/clippy 干净。变异 4 个:去掉聚焦强制后的探针作废、去掉"项目没状态不入队"、去掉项目在开检查均让测试变红;`apply_addr_focus` 里"没变化提前返回"是等价变异(省一次空写),保留作意图说明。**设计取舍(实现时定的):** ① 后退/前进/刷新只入队(`take_nav`),Task 7 的原生层取走执行;② v1 **不做**页签拖拽换位和收藏夹分隔条拖拽(Digger 壳没有通路,`DragHover`/`ColumnDragStart` 直接忽略,侧栏占 28% 固定);③ 菜单动作作用于地址栏**屏幕上显示的那份**文本(聚焦时是草稿,否则是当前网址),未聚焦时粘贴=替换整条、已聚焦时追加;④ 页签悬停动画由同一个 16ms 探针推进(Digger 之前 D2 约定无悬停动画,这里按库接口透传,验收时看观感)。**需要真实窗口验收(自动测不了):** 点击/Tab 进出地址栏后焦点状态是否一帧内正确、单击地址栏是否全选整条、右键菜单四项、收起/切走面板后地址栏不再吞终端键盘。此提交后面板有页签栏/地址栏/收藏夹但**没有网页**(Task 7)。下面的勾选项保留作记录。

**Interfaces:**
- Consumes:库 `State`/`Message`/`update`/`view`/`BrowserDisplay`/`addr_field_id`;byteui `panel_tabs`;`dozer_client::Client`(`BookmarkService`);G1 的"整框语义"菜单模式(`conversations_search_menu.rs`)。
- Produces:`BrowserPanel { states: HashMap<i64, State>, … }`,`update(pid, Message, project) -> Task<Msg>`,`view(project, area)`,`close_project(pid)`。

- [ ] **Step 1: 失败测试(`BrowserDisplay`)**:`tab_uses_byteui_panel_tab_and_carries_hover_and_tooltip`(用 byteui 部件,悬停值透传)、`region_projection_uses_digger_tokens`、`bookmarks_divider_only_built_when_open`(沿用库已有的 fake 思路)。
- [ ] **Step 2: 失败测试(焦点桥,Review Focus 5)** `addr_focus_bridge_sets_state_once_per_change`、`select_all_runs_once_after_focus_gain_and_not_again`:点击或按键后触发 `operation::is_focused(addr_field_id())`,结果经 `State::set_addr_focused` 写入;`take_addr_select_all_pending()` 为真时发一次 `operation::select_all`。S4 已验证 `focus`/`is_focused` 在标准 iced 下可用;**触发时机**(鼠标点击 / 键盘 / 失焦之后是否一帧内读到正确值)需要真实窗口,放验收清单。
- [ ] **Step 3: 失败测试(地址栏菜单)** 复用 G1 的 `SearchEdit` 形状:剪切/复制/粘贴/全选,整框语义,文本来自 Task 3 的 `addr_text()`;`Message::TextInputMenuOpen` 弹原生菜单(仅 macOS)。收藏星与页签溢出也走原生菜单(`tab_overflow_items`、库的 `StarClick` 菜单)。
- [ ] **Step 4: 实现。** 日志、错误走 Digger 现有 notice 机制(库的 `State.error` 是内联持久错误,留在原位)。**提交。**

### Task 7: 原生层(`browser_native.rs`)

**已完成(2026-10-28,Digger 本地):** `adapters/browser_native.rs`(几何、`Desired`/`prepare` 状态机、事件翻译)+ `BrowserPanel::{desired,nav_targets,sync_native,update_native}` + `product.rs` 接线(`Msg::{BrowserNative,BrowserNoticeDismiss}`、事件流、`sync_native` 里同步、16ms 探针里 `poll_stalled`)。Digger 877 通过/0 失败(+23),fmt/clippy 干净。变异 5 个(忽略停放、换项目不开新一代、PageLoaded 不校验代号、手写 chrome 高度、导航不按项目过滤)均让测试变红。**与计划原文的偏差:** ① 网页矩形的左右下边距取自视图用的区域样式(`region().padding`),不是 Dozer 的 8/16 常数——Digger 的区域底边距不同;高度只认 `chrome_top_px()`,钉死测试守着;② 面板不可见 / 有 iced 浮层是**停放**(隐藏、保留页面)而不是销毁,换项目才销毁重建;③ 新增提示条(页面没响应、网址被挡),占的高度从网页矩形里扣;④ 池事件走标准库通道,Digger 里用一个小桥线程转进 iced 的流。**冒烟抓到的真 bug(已在 bytehost 分支 `fix/web-child-self-navigation` `8714771` 修复,尚未合并 / 发版):** 页面自己跳转后宿主把 `PageLoaded` 的网址回灌,池里记的还是旧地址,下一次同步会把同一页再导航一遍;修复后实测回灌不再多一次加载,被挡网址也只报一次。**已发布(2026-10-28):** bytehost `v0.4.16`(`8714771`)、Dozer `libs-2026-10-28f`(`8a65d1a7`)已推送,Digger 本地重钉(877 通过/0 失败)。 已知局限:Digger 里测不了真实窗口的几何是否与视图排版逐像素一致(`chrome_top_px()` 是否等于 Digger 页签栏 + 地址栏的实际高度),落验收 I 区。

**Interfaces:** Consumes `bytehost_desktop::web_child::{WebPool, WebTabSpec, WebEvent, WebNav}`;库 `State::desired_webviews()`、`chrome_top_px()`。Produces:`BrowserNative`(`desired(rect, visible, generation)`、`take_receiver()`、`nav(NavAction)`),`Event`。

- [ ] **Step 1: 失败测试(几何)** `webview_rect_insets_by_chrome_top_px_and_margins`:`y = content.y + chrome_top_px()`、`h = content.h − chrome_top_px() − 8`、`x = content.x + 8`、`w = content.w − 16`(与 Dozer `webview_geometry.rs` 的 `PanelKind::Web` 分支同式),缩放 1 与 2;并**读 `browser_native.rs` 的生产源码钉住**只读 `chrome_top_px()`、无手写像素常数(同 Dozer 的几何钉死测试)。
- [ ] **Step 2: 失败测试(可见性与生命周期,Review Focus 3、4)** `hidden_when_panel_not_visible`、`iced_overlays_hide_the_webview_but_native_menus_do_not`、`leaving_the_panel_destroys_or_parks_every_webview`、`project_switch_destroys_the_previous_projects_webviews`、`closing_the_last_tab_keeps_one_blank_webview`、`stale_generation_events_are_ignored`(项目关闭后迟到的页面事件不得改任何状态)。
- [ ] **Step 3: 失败测试(事件映射)** `page_loaded_becomes_Loaded`、`title_changed_becomes_TitleLoaded`、`new_tab_request_becomes_OpenUrl_only_if_allowed`、`nav_back_forward_use_history_and_refresh_uses_reload`(后退/前进用 `window.history.back()/forward()` 经宿主侧 `evaluate_script`,刷新用原生 `reload()`,与 Dozer 一致;**不向页面注入任何脚本**)。
- [ ] **Step 4: 实现并提交。**

### Task 8: `product.rs` 接线

**已完成(2026-10-28,Digger 本地 `86aa99b`):** 大部分接线在 Task 6/7 里已随各自功能完成(`Msg` 变体、事件流、`sync_native`、探针订阅、项目关闭钩子、目录登记)。本 Task 补的:① 浏览器第一次显示时 `install_standard_edit_menu()`(macOS、幂等,`on_visible` 的浏览器分支;测试钉住只出现一次且在该分支内);② `on_current_project_changed`:面板已显示时切项目,新项目立刻建状态;③ 5 条产品级测试(目录登记、面板切入 / 切项目建状态、消息按信封项目路由、关项目清理、Edit 菜单钉死),2 个变异(去掉切项目激活、去掉 Edit 菜单)均变红。Digger 882 通过/0 失败,fmt/clippy 干净,`check-host-deps`(含 `--self-test`)、`check-shared-deps`、`check-shell-regions` 全过;新文件均已入库,无被 `.gitignore` 吞掉的。**未做(与计划原文不同):** 收藏夹分隔条拖动——Task 6 已定 v1 固定 28% 不可拖,故没有 `bookmarks_split` 换算;悬停动画 tick / tooltip 唤醒由 Task 6 的 16ms 探针承担,没有另开订阅。真实窗口的 webview 层级、焦点、输入法、Edit 菜单对 iced 输入框的回归,落验收 I 区(Task 10)。

- [ ] **Step 1: 失败测试(产品级,用 fake 原生层,仿 `group_chat_native_tests.rs`):** 打开面板→创建 webview;切页签→只有激活者可见;`OpenUrl` 经 `Msg::Browser` 路由到正确的项目;关闭项目→`close_project` 清理;收藏夹分隔条拖动(库发 `ColumnDragStart`,宿主用鼠标移动换算 `bookmarks_split`)夹在合理范围且不为 NaN;`shell.json` 里面板被收起/隐藏时 webview 隐藏。
- [ ] **Step 1b: Edit 菜单(S2 实测的必要条件):** Digger 自己不装主菜单,浏览器面板第一次显示时(或应用启动时)调 `install_standard_edit_menu()`,macOS 限定、幂等。**已在探针里确认(用户 2026-10-28):** 装了菜单后 iced 输入框的 ⌘ 组合键照常可用、webview 里 ⌘A 能全选,所以可以直接接入;验收 I 区仍保留一条回归。
- [ ] **Step 2: 实现:** `Msg::Browser(i64, Message)`、`Msg::BrowserNative(Event)`;`panel_visible`/激活钩子;几何 rect 进 `content_of(&shell::browser_id())`;订阅(原生事件 + 悬停动画 tick + tooltip 唤醒);关项目清理;重试/失败进 notice。
- [ ] **Step 3:** Digger 干净快照整套门禁(build/test/clippy/fmt、`check-host-deps.sh`(含 `--self-test`)、`check-shared-deps.sh`、`check-shell-regions.sh`)。提交。

### Task 9: 示例与文档

**已完成(2026-10-28,Digger 本地 `c5032e7`):** `crates/desktop/examples/browser_gallery.rs`(5 个状态:blank / two-tabs / bookmarks-open / notice / inline-error;`--list`、`--state`、`--check`;合成收藏夹,无真实 webview;三个状态实际启动并保持运行,**未能截图**——没有屏幕录制权限,视觉效果没看);`docs/adr/ADR-016-browser-panel-untrusted-content.md`;`docs/superpowers/notes/2026-10-28-browser-open-items.md`;README 加 ADR-016 与画廊命令。Digger 882 通过/0 失败。**ADR 里如实标为未决:** 最低 macOS 版本(只在 26.6.2 上测过)、定位权限未测、`file://` 页面除 `fetch` 外读本地文件的路径未测。`--check` 契约读不到库的内联错误,`inline-error` 只能在窗口里看。

- [ ] `examples/browser_gallery.rs`(仿其它 gallery,无真实 webview,看视图);ADR-016 写明不可信内容的 wry 设置、导航白名单、`file://` 的取舍与已知风险(**本地文件页面能读同源的其它本地文件**,不是宿主权限,但用户要知道);Digger 的 open-items 与 README 里面板清单更新。提交。

### Task 10: 验收清单与收口

- [ ] 在 `digger/docs/superpowers/reviews/2026-10-28-s7-s8-closeout-acceptance.md` 加 **I 区「Digger 浏览器面板」**(沿用 H 区的 15 项,外加):`file://` 打开(含空格/中文路径)、页面请求摄像头/定位**不被静默批准**、**Edit 菜单装上后,iced 输入框(地址栏、会话搜索框、群聊输入)里的 ⌘C/⌘V/⌘X/⌘A 仍然可用**、页面里 ⌘A 真的全选、⌘C/⌘V/⌘X 在页面里生效、点页面里任何 `dozer://`/`javascript:` 链接**不跳走**、下载被拒、`target=_blank` 开新页签、切项目后旧页面被销毁、iced 浮层(收藏星菜单)打开时网页不盖住它、地址栏中文输入法、⌘C/⌘V 在页面里、关最后一个页签回到空白页、webview 顶边紧贴地址栏下沿(缩放 1 与 1.5)、`file://` 页面的同目录图片/样式能加载、页面读不到其它本地文件(`fetch` 失败)、**导航失败(不可达地址)有明确提示而不是地址栏已变页面不变**、点击 / Tab / 失焦后地址栏焦点与全选的时机。
- [ ] 记忆:当前版本组与"Digger 浏览器面板"状态;**请用户目测验收后停下**。

---

## 范围外(明确不做)

- 把 Dozer 的浏览器 webview 迁到 `web_child`(Dozer 现在装着 `dozer://` 协议和完整 IPC 的预览 builder 去承载网页,**这是既有的安全隐患,但不是本计划**;建议另立一份计划评估)。
- ⌘F 页内查找、下载管理、书签导入导出、历史记录页面、多窗口、扩展、无痕模式、自定义 User-Agent。
- 非 macOS 的功能(只要求能编译)。

## 顺序与依赖

Task 0 → Task 1 → Task 2 → Task 3 → Task 4(发版联动)→ Task 5 → Task 6 → Task 7 → Task 8 → Task 9 → Task 10。Task 5(`shell.rs` 注册)与 Task 1–3 互不依赖,可并行,但**注册面板之前必须先有能用的原生层**,否则用户打开一个空面板。我没有可靠的工时依据,不给估计。

## 仍然未决的未知(探针之后)

1. **旧 macOS 上含空格 URL 会不会 panic**——没有旧系统可测;已用"宿主自己百分号编码"绕开,并要在 ADR 写明最低支持版本。
2. **定位权限**——wry 没有钩子,未测;若面板要对外使用,上线前人工确认。
3. **导航失败**——wry 无回调,靠超时兜底,需要定超时秒数与文案(设计决定,实现前问用户)。
