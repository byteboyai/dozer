# S3b-1 Digger 待办面板接入 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Digger 出现与 Dozer 一致的待办面板(分类树 + 任务列表 webview + 新增输入),数据来自 `diggerd`,增删改查与指派 agent 可用。

**Architecture:** 复用共享库 `dozer-panel-todo`(Dozer 已发 tag `libs-2026-10-24e`)。Digger 新增三块适配:`adapters/todo.rs`(每项目一份库状态 + 服务 + 消息分派)、`adapters/todo_native.rs`(wry 子视图 + 自定义协议 `digger-todo://` + `WebviewPushState`)、`adapters/todo_assets.rs`(嵌入资源)。`shell.rs` / `product.rs` 注册面板并把它放进配对布局。不改库、不改 Dozer、不改 `diggerd`。

**Tech Stack:** Rust、iced 0.14、wry、tokio、`dozer-panel-todo` / `dozer-panel-host` / `dozer-services` / `dozer-client`(均按 git tag)

**Spec:** `docs/superpowers/specs/2026-10-25-digger-todo-panel-s3b-design.md`(决定 D1–D6 与 §7 的默认值在这里;本计划按这些默认值写)

**工作位置:** Digger 仓库 `/Users/chrischiang/Projects/CoralProjects/byteboy/digger`,**独立分支**(如 `feat/todo-panel`),在独立 worktree 里做;不碰 `main` 工作区里的未提交改动、不碰 `feat/preview-pairing`。

## Global Constraints

- **库 tag 一致**:新增依赖与现有 `dozer-panel-*` 同为 `tag = "libs-2026-10-24e"`;`scripts/check-shared-deps.sh` 必须通过(byteui `v0.4.6`、bytehost `v0.4.12`、bytegit `v0.8.1`)。
- **严格 CSP 不放宽**:待办页面只能加载 `digger-todo://localhost/` 下的三个文件;导航只放行该前缀;下载拒绝、新窗口拒绝、devtools 关。
- **不用 `thread_local` 存订阅回调要读的状态**(用进程级原子);wry 视图句柄的 `thread_local` 只在 `window::run` 闭包里访问(主线程),与 `usage_native.rs` 同。
- **字体**:UI 文本一律系统默认字体(`Font::default()`),只有代码编辑器和终端用 JetBrains Mono。
- **没有 Toast**:面板提示落 stderr(与 Project 面板一致),**不要**为此新造提示机制。
- **不碰真实数据**:测试一律用独立 `DIGGER_DATA_DIR` 和临时 `HOME`;不碰用户常驻的 `diggerd`。
- **commit 前看 staged 全貌**;`Cargo.lock` 里本地 `[patch]` 造成的改动不提交。
- **验收测试在干净快照里跑**(`git archive` 导出到不含 "digger" 的路径),避免被 `.gitignore` 吞掉的文件造成假绿。
- 提交信息末尾带:`Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`

## Review Focus

规格沉默但真人一定会碰到的情况,每条都在对应任务里有测试:

1. **`diggerd` 还没起来 / 中途挂了再恢复**(Task 2):面板读失败为 `Unavailable` 时保持静默、保留上一份列表,恢复后下一次激活自动刷新;不弹错、不清空已显示的任务。
2. **快速切项目时在途请求晚到**(Task 2):项目 A 的 `Loaded` 在已切到项目 B 之后才到,必须丢弃,不能写进 B 的列表。
3. **webview 与 iced 控件重叠**(Task 4):列表栏收起、窗口很窄、面板整体收起三种情况下,webview 矩形都不得盖住底部新增输入框;面板不可见时 webview 必须销毁(不留在别的面板上方)。
4. **任务文字里的特殊字符**(Task 3):中文、emoji、换行、引号、`</script>`、` ` 的任务文字经 `evaluate_script` 推给页面后原样显示,不截断、不执行。
5. **升级前保存的布局文件没有 `digger.todo`**(Task 4):旧布局能加载,待办被追加到默认侧末尾,用户已有的面板顺序不变。
6. **页面回传的 id 已不存在**(Task 5):另一个进程(Dozer / MCP)刚删掉这条任务,页面还拿着旧 id 发来 `Toggle`,必须忽略,不报错不崩。

---

### Task 1: 依赖与资源管线

**Files:**
- Modify: `Cargo.toml`(workspace 依赖表)、`crates/desktop/Cargo.toml`(`[dependencies]` 与 `[build-dependencies]` 各加 `dozer-panel-todo`)、`crates/desktop/build.rs`
- Create: `crates/desktop/src/adapters/todo_assets.rs`;在 `adapters/mod.rs` 注册
- Modify(若门禁要求):`scripts/check-host-deps.sh` 的白名单——`dozer-panel-todo` 会和 `dozer-panel-usage` 一样把 `iced_renderer` 带进构建脚本单元,按 Cargo.toml 里对 usage 的那段说明处理

**Interfaces:**
- Produces: `pub fn todo_assets::serve(path: &str) -> Option<(&'static str, &'static [u8])>`,`path` 取自请求 URI 的 path 部分(以 `/` 开头),返回 `(content-type, 字节)`;只认 `/host.html`、`/todo-content.js`、`/todo-content.css`。

- [ ] **Step 1: 写失败测试**(`todo_assets.rs` 的 `#[cfg(test)]`)
  - `serves_the_three_files_with_their_mime_types`:`serve("/host.html")` 的类型是 `text/html; charset=utf-8`,`/todo-content.js` 是 `text/javascript; charset=utf-8`,`/todo-content.css` 是 `text/css; charset=utf-8`,三个字节串都非空。
  - `unknown_paths_are_not_served`:`serve("/nope")`、`serve("/../usage-content.js")`、`serve("/")` 都是 `None`。
  - `host_page_has_a_strict_csp_and_no_external_refs`:`host.html` 含 `default-src 'none'`,不含 `http://`、`https://`、`//cdn`。
  - `todo_and_usage_host_pages_are_different_files`:待办的 `host.html` 字节与用量的(`usage_native` 里同名嵌入)**不相等**,且待办页含 `todo-content.js`(规格 D3)。
- [ ] **Step 2: 跑测试确认失败**:`cargo test -p digger-desktop --lib todo_assets`,预期编译失败(模块不存在)。
- [ ] **Step 3: 实现**:`build.rs` 对三个文件各 `fs::copy` 到 `OUT_DIR`,目标名加前缀:`todo-host.html`、`todo-content.js`、`todo-content.css`(`rerun-if-changed` 同用量);`serve` 用 `include_bytes!(concat!(env!("OUT_DIR"), "/todo-host.html"))` 等。用量那部分的拷贝不动。
- [ ] **Step 4: 跑测试确认通过**,再跑 `bash scripts/check-shared-deps.sh && bash scripts/check-host-deps.sh && bash scripts/check-host-deps.sh --self-test`,预期全过。
- [ ] **Step 5: Commit** `feat(todo): 引入 dozer-panel-todo 并嵌入其 web 资源`

### Task 2: `TodoPanel` 适配器(状态、服务、消息分派)

**Files:**
- Create: `crates/desktop/src/adapters/todo.rs`;在 `adapters/mod.rs` 注册
- Modify: `crates/desktop/src/adapters/project.rs`:把私有的 `UnavailableService` 提成 `pub(crate)`(或移到 `adapters/services.rs`),供待办复用(规格 D5)

**Interfaces:**
- Consumes: `dozer_panel_todo::{WorkspaceState, Message, update, on_activate, adapter::PanelDisplay}`、`dozer_panel_host::{ActivationCtx, PanelIo}`、`dozer_services::{Services, TodoService}`、`dozer_client::Client`
- Produces(`adapters/todo.rs`):
  - `pub struct TodoPanel`,字段含 `states: HashMap<i64, WorkspaceState>`、`current: Option<i64>`、`service: Arc<dyn TodoService>`、`client: Client`、`handle: Option<tokio::runtime::Handle>`、`tx` / `rx`(类型 `UnboundedSender/Receiver<dozer_panel_todo::Message>`,项目归属由消息里的请求令牌带,用 `Message::result_project()` 取)、`pub native: todo_native::Native`(Task 3 提供;本任务先用 `Option<()>` 占位或把字段放到 Task 3 再加)
  - `pub fn new(service: Arc<dyn TodoService>, client: Client) -> Self`
  - `pub fn attach_handle(&mut self, handle: tokio::runtime::Handle)`
  - `pub fn take_receiver(&mut self) -> Option<UnboundedReceiver<dozer_panel_todo::Message>>`
  - `pub fn set_project(&mut self, project_id: Option<i64>)`:切当前项目(不存在则建一份空状态)
  - `pub fn activate(&mut self, project_path: Option<PathBuf>)`:对当前项目调库的 `on_activate`;无当前项目或无句柄时什么都不做
  - `pub fn update(&mut self, msg: dozer_panel_todo::Message) -> Vec<TodoEffect>`
  - `pub enum TodoEffect { ToggleListCollapse, AssignAgent { todo_id: i64, agent: dozer_model::AgentKind } }`
  - 关联常量 `TodoPanel::POLL_INTERVAL: Duration = Duration::from_millis(1000)` 与 `pub fn poll(&mut self, now: Instant)`:面板可见时的定时刷新,与 Dozer 的 `TODO_POLL_INTERVAL`(1 秒,`window_events.rs:2915`)同值同语义——自限速(距上次不足 `POLL_INTERVAL` 就什么都不做),对当前项目调库的 `request_todos_refresh` 与 `request_categories_refresh`。**没有它,agent 经 MCP 改的任务不会出现在列表里**(S3b-2 的前提)
  - `pub fn take_notices(&mut self) -> Vec<(String, String)>`:排空库的发件箱,与 `UsagePanel::take_notices` 同形
  - `impl PanelDisplay for` 一个内部的 `Display<'_>`(借用 `HoverTracker`/折叠状态),`locale()` 恒 `"en"`

- [ ] **Step 1: 写失败测试**(`todo.rs` 的 `#[cfg(test)]`,测试里自带 `FakeTodo`:实现 `TodoService` 的全部方法,`list_todos` / `list_categories` / `add_todo` / `toggle_todo` 走内存,其余返回 `ServiceErrorKind::Unavailable`;可设"读失败"开关)
  - `activate_loads_todos_and_categories_for_the_current_project`:`set_project(Some(7))` + `activate` 后,经 `rx` 收到的消息喂回 `update`,`states[&7].items()` 是 FakeTodo 的数据。
  - `results_for_a_previous_project_are_dropped`(Review Focus 2):A 项目发起 `activate`,**先** `set_project(Some(B))`,**再**把 A 的结果喂进 `update`;B 的状态不变,A 的状态也不被写(请求令牌已作废)。
  - `unavailable_reads_keep_the_last_list_and_stay_silent`(Review Focus 1):先成功加载 3 条,再让 FakeTodo 读失败(`Unavailable`),喂 `LoadFailed` 后 `items()` 仍是 3 条,`take_notices()` 为空;恢复后再 `activate`,列表更新。
  - `non_unavailable_read_failures_produce_a_notice`:读失败 kind 为 `Internal` 时 `take_notices()` 恰有一条,文本非空。
  - `every_library_message_has_an_explicit_disposition`:一个穷尽 `match`(无 `_` 分支)覆盖库 `Message` 的**所有**变体,每个映射到 `Handled` / `Effect` / `Ignored(原因)`;库新增变体会让这段代码编译失败。`ToggleListCollapse` → `TodoEffect::ToggleListCollapse`;`TextInputMenuOpen`、`DetailClose`、`DetailReplyInput`、`DetailReplySubmit` → `Ignored`(规格 §4 非目标);`ClearListRequest` / `ClearListCancel` / `ClearListConfirm` → 交给库的 `update`(弹窗流程在库里)。
  - `states_are_kept_per_project`:切 7 → 8 → 7,7 的列表还在,没有重新置空。
  - `poll_is_rate_limited_and_only_refreshes_the_current_project`:连续两次 `poll(t)`、`poll(t + 200ms)` 只发一轮请求(FakeTodo 记录 `list_todos` 次数为 1);`poll(t + 1s)` 再发一轮;没有当前项目或没有句柄时一次都不发。
- [ ] **Step 2: 跑测试确认失败**
- [ ] **Step 3: 实现**:`update` 先按 `msg.result_project()`(无则 `current`)选状态,再调 `dozer_panel_todo::update(state, msg, project_id, &self.service, handle, emit)`,`emit` 把消息投进 `tx`;无句柄时只记状态不起请求(同 `UsagePanel::activate` 的处理)。服务束 `Services { todo: self.service.clone(), 其余 UnavailableService }` 只用于构造 `ActivationCtx` 的 `PanelIo`。
- [ ] **Step 4: 跑测试确认通过**,`cargo clippy -p digger-desktop --all-targets -- -D warnings`
- [ ] **Step 5: Commit** `feat(todo): TodoPanel 适配器(按项目状态、服务、消息分派)`

### Task 3: `todo_native` —— wry 宿主与推送

**Files:**
- Create: `crates/desktop/src/adapters/todo_native.rs`;在 `adapters/mod.rs` 注册;`TodoPanel` 加字段 `pub native: todo_native::Native`

**Interfaces:**
- Consumes: `todo_assets::serve`、`dozer_panel_todo::{WebviewPushState, parse_todo_event, TodoWebviewEvent, current_view_payload, encode_todo_push}`、`dozer_model::{Stamped, LocaleStamp, Locale}`
- Produces:
  - `pub enum Event { Ready(u64), Built(u64, Result<(), String>), Todo(u64, TodoWebviewEvent), Done }`(`u64` 是 epoch)
  - `pub struct Native`,`pub fn new() -> Self`、`pub fn take_receiver(&mut self) -> Option<UnboundedReceiver<Event>>`、`pub fn update(&mut self, event: Event) -> Option<TodoWebviewEvent>`(只在 epoch 匹配且 webview 存活时把页面事件交出去;`Ready`/`Failed` 自己消化进 `WebviewPushState`)、`pub fn sync(&mut self, window: iced::window::Id, visible: bool, rect: LogicalRect, state: &WorkspaceState) -> Task<Event>`、`pub fn failed(&self) -> Option<String>`
  - 自定义协议名 `digger-todo`,入口 `digger-todo://localhost/host.html?theme=dark`(页面从 `?theme=` 取主题,缺省 dark;Digger 固定 dark)

- [ ] **Step 1: 写失败测试**
  - `invisible_destroys_the_view_and_bumps_the_epoch`:`sync(visible=false)` 后 `live == false`、epoch +1;旧 epoch 的 `Ready` / `Todo` 事件经 `update` 被丢弃(返回 `None`,`ready` 不变)。
  - `ready_forces_a_full_repush`:`Ready` 后 `WebviewPushState::pending_push` 对当前 payload 返回 `Some`;`mark_sent` 后同 payload 返回 `None`;`set_ready(true)` 再来一次又返回 `Some`(重建后重发)。
  - `locale_change_alone_triggers_a_push`:payload 不变、`LocaleStamp.locale_revision` 变了,`pending_push` 仍返回 `Some`。
  - `dispatch_script_survives_hostile_task_text`(Review Focus 4):对任务文字 `"他说:\"hi\"\n</script><img src=x onerror=alert(1)>\u{2028}😀"`,生成的脚本能被 `serde_json` 往返还原成同一字符串,且脚本文本里不含未转义的 `</script>` 与 U+2028。
  - `navigation_is_limited_to_the_todo_protocol`:导航白名单函数(从 wry 构造里抽成纯函数 `allows_navigation(&str) -> bool`)对 `digger-todo://localhost/host.html` 为真,对 `https://example.com`、`digger-usage://localhost/host.html`、`file:///etc/passwd` 为假。
- [ ] **Step 2: 跑测试确认失败**
- [ ] **Step 3: 实现**:结构照 `usage_native.rs`(epoch、`live`、`ready`、`bounds`、`thread_local VIEW`、`window::run` 里建 / 更新 / 销毁)。wry 构造:`with_url("digger-todo://localhost/host.html?theme=dark")`、`with_custom_protocol("digger-todo", ..)` 用 `todo_assets::serve`(未命中返回 404)、`with_navigation_handler(allows_navigation)`、`with_new_window_req_handler(Deny)`、`with_download_started_handler(|_, _| false)`、`with_devtools(false)`、`with_ipc_handler`:`parse_todo_event(body)` 成功则 `tx.send(Event::Todo(epoch, ev))`(`Ready` 事件同样走这条,由 `update` 识别),解析失败只记日志。`sync` 里:`set_desired` 用 `Stamped::new(LocaleStamp{ ui_locale: Locale::default_locale(), locale_revision: 0 }, current_view_payload(...))` 的等价物(以库 `current_view_payload` 的实际签名为准);`WebviewPushState::observe_availability(self.live, Instant::now())` 每帧调用以驱动加载超时;`pending_push` → `mark_sent` → `encode_todo_push(revision, Some(locale), payload)` → `dispatch` 包成 `window.__dozer?.dispatch(<json 字符串>);`(`dispatch` 的写法同 `usage_native::dispatch`,双重 JSON 编码保证转义)。
- [ ] **Step 4: 跑测试确认通过**
- [ ] **Step 5: Commit** `feat(todo): todo_native 宿主(wry + digger-todo 协议 + 推送去重)`

### Task 4: 注册面板、布局与几何

**Files:**
- Modify: `crates/desktop/src/shell.rs`(`todo_id()`、`all_panels()`、`meta()`、`rules()`、`defaults()`)、`crates/desktop/src/product.rs`(`Msg::Todo(dozer_panel_todo::Message)`、`Msg::NativeTodo(todo_native::Event)`、`Msg::TodoPoll`(订阅里按 `TodoPanel::POLL_INTERVAL` 的定时器,只在待办面板可见时订阅;处理时调 `todo.poll(Instant::now())`)、`ProductShell.todo: TodoPanel`、`todo_view`、`sync_native` 里的待办原生视图、`on_visible` / 切项目钩子、`body()` 里 `"digger.todo" => self.todo_view(area)`、`take_notices` 排空、`boot` 里接 `take_receiver` 成 iced 流并 `attach_handle`)
- Modify: `crates/desktop/src/adapters/todo.rs`(`list_view` / `content_view` 取库 `view(...)` 的两块;`PanelDisplay` 的折叠与悬停对接 `PaneStore`)

**Interfaces:**
- Produces:
  - `shell::todo_id() -> PanelId`(`"digger.todo"`);`meta` 的标题 `"待办"`、图标 `IconKind::ListTodo`(库视图头用的同一图标);`rules`:与 `"digger.project" | "digger.files"` 同一组(`default_side: Side::Left`、`PaneSpec::new(true, true)`、`min_first_px: list_min_width()`);`defaults()` 的左区顺序 `[project, files, todo, markdown, info]`
  - `product::todo_native_rect(content: Rect, pair: Option<PairRects>) -> Option<Rect>`:原生视图的矩形 = 配对的第二块(内容区)扣掉**库视图为 iced 控件预留的区域**(顶部行 `dozer_panel_todo::adapter::top_row_inner_h_px()` 加头部内边距,底部 `WorkspaceState::add_input_height()`);列表栏收起时内容区独占整块;整个面板收起返回 `None`

- [ ] **Step 1: 写失败测试**
  - `shell.rs`:`todo_is_a_left_pair_like_project_and_files`(规则同 project/files);`all_panels_contains_todo_and_meta_exists`;`a_saved_layout_without_todo_still_loads_and_appends_it`(Review Focus 5:给一个只含 `digger.project`/`digger.files`/`digger.markdown`/`digger.info` 与右区 `agent/apps/usage` 的旧布局 JSON,`load` 后每个旧面板的相对顺序不变,`digger.todo` 出现在默认侧末尾,仿照 usage 的同类测试 `an older file that predates the usage panel`)。
  - `product.rs`:
    - `todo_native_rect_never_overlaps_the_add_input`(Review Focus 3):对 (列表展开 / 列表收起 / 窗口宽 700 / 窗口宽 1600) 四种内容区,`rect.bottom + add_input_height <= content.bottom` 且 `rect.top >= content.top + top_row_inner_h_px`。
    - `todo_native_is_destroyed_when_the_panel_is_not_visible`:面板收起或切到别的面板时 `sync_native` 对待办传 `visible=false`(Task 3 已保证此时销毁)。
    - `switching_project_pushes_the_new_projects_payload_without_rebuilding_the_view`:切项目后 `native.live` 仍为真、epoch 不变,`pending_push` 对新项目的 payload 返回 `Some`。
    - `the_poll_tick_is_subscribed_only_while_the_panel_is_visible`:`subscription(shell)` 在待办面板可见时包含一个 `iced::time::every(TodoPanel::POLL_INTERVAL)`(映射成 `Msg::TodoPoll`),不可见 / 收起 / 关闭中时不包含(省电,同 Dozer `poll_todo_if_visible` 的可见性前提)。
    - `todo_message_variants_route_into_the_panel`:`Msg::Todo(m)` 经 `TodoPanel::update`,`TodoEffect::ToggleListCollapse` 翻转 `PaneStore` 里待办面板的列表折叠。
- [ ] **Step 2: 跑测试确认失败**
- [ ] **Step 3: 实现**:`ProductShell::boot` 里用 `dozer_client::Client::new(digger_paths::product().socket_path())` 构造 `TodoPanel::new(Arc::new(client.clone()), client)`;切入面板(`on_visible`)与换项目页签时调 `todo.set_project(Some(project.id))` + `activate(Some(path))`;无项目时 `set_project(None)`、内容区显示库的空态。`todo_view` 仿 `usage_view`:`pair_view(list, content)`,列表 = 库 `view` 的第一块,内容 = 第二块,`webview_failed` 取 `native.failed()`。几何的"库预留区"必须取库导出的函数,**不手写像素常数**。
- [ ] **Step 4: 跑测试确认通过**,再跑整个 `digger-desktop` 的测试看没有回归
- [ ] **Step 5: Commit** `feat(todo): 注册待办面板、配对布局与原生视图几何`

### Task 5: 页面事件路由与指派 agent

**Files:**
- Modify: `crates/desktop/src/product.rs`(`Msg::NativeTodo` 的处理)、`crates/desktop/src/adapters/todo.rs`

**Interfaces:**
- Consumes: `dozer_panel_todo::{route_event, Routed}`
- Produces: `TodoPanel::handle_webview_event(&mut self, event: TodoWebviewEvent) -> Vec<TodoEffect>`;`TodoPanel::assign_agent(&self, todo_id: i64, agent: AgentKind)`:经 `client.assign_todo_agent(id, agent)` 异步调用,结束后投 `Message::AsyncResult(请求令牌, Box::new(Message::Mutated(结果)))` 触发刷新(与 Dozer `App::todo_assign_agent` 同款:先 `requests.invalidate("todos")` 再取令牌)

- [ ] **Step 1: 写失败测试**
  - `page_events_become_the_same_messages_as_dozer`:对 `Add{text}` / `Toggle{id}` / `EditText` / `Reorder` / `SetStatus` / `SetPlanDate` / `SetCategory` 各一条,`handle_webview_event` 之后 `FakeTodo` 收到对应的服务调用(用 FakeTodo 的调用记录断言)。
  - `an_event_for_a_task_that_no_longer_exists_is_ignored`(Review Focus 6):状态里只有 id 1、2,发 `Toggle{id: 99}` 与 `EditText{id: 99, ..}`:无服务调用、无 `TodoEffect`、不 panic。
  - `events_for_optimistic_placeholder_ids_are_ignored`:乐观新增的临时项(`id <= 0`)收到 `Toggle` 被忽略(库的 `route_event` 已保证,这里钉住不被产品层绕过)。
  - `assigning_an_agent_calls_the_client_and_refreshes`:`AssignAgent{id, agent}` → `TodoEffect::AssignAgent`;`assign_agent` 之后收到 `Mutated(Ok)` 并触发重新加载(用一个本地 UDS 假服务端或把 `assign` 抽成可注入的闭包来测,二选一,在 PR 里写明)。
  - `open_detail_is_a_documented_no_op`:`OpenDetail{id}` 返回空 effect 列表,且不产生 stderr 以外的副作用(规格 Q2)。
  - `add_height_events_resize_the_native_rect`:`AddHeight{px}` 更新 `WorkspaceState::add_input_height`,下一次 `todo_native_rect` 的底边随之上移(Task 4 的几何保持成立)。
- [ ] **Step 2: 跑测试确认失败**
- [ ] **Step 3: 实现**:`Msg::NativeTodo(ev)` → `native.update(ev)` 取出 `TodoWebviewEvent` → `handle_webview_event` → `route_event(state.items(), event)`:`Routed::Message(m)` 进 `update`;`Routed::AssignAgent` → `assign_agent`;`Routed::OpenDetail` → 只写一行 stderr(`todo detail not available in Digger yet`)后忽略。
- [ ] **Step 4: 跑测试确认通过**
- [ ] **Step 5: Commit** `feat(todo): 页面事件路由与指派 agent`

### Task 6: 对真实 `diggerd` 的集成测试、真机清单与门禁

**Files:**
- Create: `crates/desktop/tests/todo_panel_daemon.rs`(沿用 `examples/product_daemon` 的拉起方式:独立 `DIGGER_DATA_DIR`、临时 socket,测试结束杀进程)、`docs/superpowers/reviews/2026-10-25-todo-s3b1-acceptance.md`(真机清单)
- Modify: `docs/superpowers/` 里相关状态行

**Interfaces:**
- Consumes: Task 2–5 的 `TodoPanel`;`Client::new(socket)`;`digger_paths::product()` 的覆盖手段(同现有集成测试)

- [ ] **Step 1: 写测试** `todo_panel_daemon.rs`:
  - `panel_round_trips_against_a_real_diggerd`:起 `diggerd`,经 `ProjectHost` 建一个项目拿到 `project_id`;`TodoPanel` `set_project` + `activate`,等 `rx` 回消息并喂 `update`;经 `update(Message::Add..)` 或 `handle_webview_event(Add{text:"买牛奶"})` 新增;再 `activate` 一次;断言 `items()` 含"买牛奶",且**另起一个 `Client` 直接 `list_todos(project_id)`** 能读到同一条(证明落进了 `digger.db` 而不是只在内存)。
  - `todos_of_two_projects_do_not_mix`:两个项目各加一条,各自 `items()` 只含自己的。
  - `daemon_restart_keeps_the_todos`:杀掉 `diggerd` 重起,再 `activate`,任务还在。
- [ ] **Step 2: 跑测试**:`cargo test -p digger-desktop --test todo_panel_daemon`,预期通过(`CARGO_TARGET_DIR` 不与别的工作区共享,避免 `product_daemon` 示例路径硬编码问题)。
- [ ] **Step 3: 写真机清单**(`2026-10-25-todo-s3b1-acceptance.md`),用户在**沙箱 Digger**(独立数据目录和临时 HOME)里逐项勾:
  1. 面板出现在左区图标栏,图标与悬停标题"待办"正确;点开是"左分类树 | 右列表"。
  2. 新增一条任务(中文、emoji、很长一行、含换行各一条),列表立即出现,重启 Digger 后还在。
  3. 勾选 / 取消勾选、改文字、拖动排序、定计划日期、改状态(待办 / 搁置 / 完成),每项刷新后保持。
  4. 分类:新建根分类与子分类、重命名、拖动换父级、上移下移、删除;任务挂到分类与移出。
  5. 指派 agent:任务卡上选 Claude / Codex,刷新后徽标保持(**不会执行**,见规格 Q3)。
  6. 切项目:两个项目各有各的任务,来回切不串。
  7. 收起 / 展开列表栏、拖窄窗口:webview 不盖住底部新增输入和清空栏;收起整个面板后没有残留的 webview。
  8. 停掉 `diggerd` 再起:面板不报错、列表不清空,恢复后数据更新。
  8b. 面板开着时,在另一个终端对同一个 `diggerd` 加一条任务(`Client::add_todo`,或 Task 6 的集成测试里用过的小工具),约 1 秒内出现在列表里;面板收起后不再轮询(`diggerd` 日志里没有周期性的 `ListTodos`)。
  9. 在另一个窗口 / 用 `digger-mcp` 之外的手段(直接对库)删掉一条任务后,页面上点那条:无反应、不崩。
  10. 已知缺位(预期如此,不算 bug):点卡片"打开详情"无反应;文本框没有右键菜单;写失败没有可见提示。
  11. 与 Dozer 并排对照同一份数据的外观(分类树、卡片、日历、颜色),截图差异逐条记。
- [ ] **Step 4: 门禁**:`cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`bash scripts/check-shared-deps.sh`、`bash scripts/check-host-deps.sh`、`bash scripts/check-host-deps.sh --self-test`、`bash scripts/check-shell-regions.sh`,然后在**干净快照**里 `cargo test --workspace`。预期全部通过;失败要贴输出,不要重跑到绿。
- [ ] **Step 5: Commit** `test(todo): 对真实 diggerd 的集成测试与验收清单`;分支交给用户审阅后合并,**合并、推送、发版由用户决定**(发版在 S3b-2)。
