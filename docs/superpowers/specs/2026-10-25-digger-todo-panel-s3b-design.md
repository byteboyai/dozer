# S3b 待办(Todo)面板接入 Digger

- 日期:2026-10-25
- 状态:设计稿,待用户审阅(§7 有 5 个待你裁决的问题,每个都给了默认值,不裁决也能开工)
- 路径:**分两期交付,每期单独一份实施计划**:S3b-1 面板(列表 + 内容区 + 增删改查)、S3b-2 MCP 待办工具(即 S6e-3b)与发布
- 上游:`2026-10-15-panel-todo-lib-design.md`(S3a,Dozer 里抽出 `dozer-panel-todo`,已完成)、`2026-10-18-agent-panel-s6-design.md` §基座与呈现的划分("Todo、群聊等面板自带业务逻辑与存储,只需接入 `diggerd`")、`2026-10-18-agent-mcp-s6e-design.md` §6.3(S6e-3b 依赖本设计)
- 实施计划:`docs/superpowers/plans/2026-10-25-digger-todo-s3b1-panel.md`、`2026-10-25-digger-todo-s3b2-mcp-and-release.md`

## 1. 用户意图

Digger 要有与 Dozer 一致的待办面板:按分类树组织任务,新增、勾选、改文字、拖动排序、定计划日期、改状态、指派 agent。库已经在 Dozer 里抽好(S3a),Digger 只做接入,**不 fork 面板逻辑**:Dozer 里改了待办面板,Digger 升一个 tag 就跟上。

## 2. 现状(读代码得到)

**已经具备的(这是本设计比预想小的原因):**

- **存储与服务端不用做。** `dozerd::daemon::run` 无条件构造 `TodoStore` 与 `CategoryStore`(`daemon.rs:117-118`),不受 `DaemonFeatures` 开关影响;`diggerd` 只是它的薄壳,用 `digger.db`。待办的全部请求(`ListTodos`、`AddTodo`、`SetTodoPlanDate`……)在 `diggerd` 里已经能用。
- **服务接缝已有生产实现。** `impl TodoService for dozer_client::Client`(`dozer-client/src/lib.rs:1951`)。Digger 桌面已依赖 `dozer-client` 并持有指向 `diggerd` 的 `Client`(`product.rs` 的预览桥就是这么建的)。
- **项目 id 一致。** 桌面进程内的 `ProjectHost` 与 `diggerd` 用同一个 `digger.db`、同一套 `projects` 表(S6a 的 D5),所以桌面算出的 `project_id` 就是 `diggerd` 里待办所属的项目 id。
- **库的公开面是完整的。** `dozer-panel-todo`:`WorkspaceState`、`Message`、`update(state, msg, project_id, &service, &handle, emit)`、`on_activate(state, &ActivationCtx<Message>)`、`view(...) -> (列表 pane, 内容 pane)`、`PanelDisplay`、`web_assets_root()`,以及 webview 的纯逻辑:`parse_todo_event`、`route_event`、`WebviewPushState`、`encode_todo_push`、`current_view_payload`。库已随 tag `libs-2026-10-24e` 发布。

**必须在 Digger 里新做的:**

| 部分 | 为什么要新做 |
|---|---|
| **webview 宿主** | 库只给协议和推送状态机,**不含 wry 视图的创建、静态资源服务和 IPC 接线**——这些在 Dozer 里散落在 `dozer-app` 的 `runtime.rs` / `assets.rs` / `app.rs`。Digger 要写自己的,照 `adapters/usage_native.rs` 的做法(独立 wry 子视图 + 自定义协议 `digger-todo://` + `build.rs` 嵌入资源) |
| **`PanelDisplay` 实现** | 悬停进度、折叠按钮、面板头、分隔线、内边距由产品提供 |
| **每项目一份 `WorkspaceState`** | 待办按项目隔离;Digger 的 `ProjectPanel` / `PreviewHub` 已有 "按 project id 存状态" 的先例 |
| **面板注册与布局** | `shell.rs` 的 id / 图标 / 配对规则 / 默认布局,`product.rs` 的消息、视图、原生视图同步 |
| **消息的逐条处置** | 库的 `Message` 里有一批"内核拦截"的变体(折叠、文本框右键菜单、详情弹窗、清空确认),Digger 必须逐条决定处理还是忽略 |

**Digger 的两个已知限制,会直接影响体验:**

- **没有 Toast**:Digger 一期把面板提示落到 stderr(`product.rs` 排空 Project 面板提示处)。待办写失败时用户**看不到**提示,只能看到列表被刷新回原样(库的 `Mutated(Err)` 会触发刷新纠正乐观更新)。
- **没有浮层窗口设施**:Dozer 的待办详情是原生浮层窗口(`platform/todo_detail_overlay.rs`),Digger 没有这套,见 §7 Q2。

## 3. 目标(S3b-1)

- G1 Digger 右侧/左侧出现"待办"面板(默认左区,配对:左 = 分类树 + 清空栏,右 = 任务列表 webview + 底部新增输入),数据来自 `diggerd`。
- G2 增、勾选、改文字、排序、计划日期、改状态、改分类、分类增删改移、指派 agent 全部可用,与 Dozer 行为一致。
- G3 切项目、隐藏/重新显示面板、`diggerd` 不可用再恢复,都不丢状态、不串项目。
- G4 webview 恒不盖住 iced 的底部输入和清空栏(几何同源)。

## 4. 非目标

- 不做待办详情弹窗(§7 Q2)。点任务卡的"打开详情"在 Digger 里暂时无反应。
- 不做"处理任务"(`ProcessTodoNow`)和自动轮询派发:它们依赖 `DaemonFeatures.task_poller` / `summaries`,Digger 的 `diggerd` 按 S6a D7 默认关闭(会调用外部 LLM、产生费用)。指派 agent 只写库里的一个字段,不触发执行。见 §7 Q3。
- 不做文本框右键菜单(`TextInputMenuOpen`):Digger 没有通用输入框右键菜单设施。
- 不做语言切换:Digger 一期固定 `en`,库的文案取英文兜底。
- 不改 Dozer 的待办行为,不改库的公开面(如发现必须改,在计划里单独列任务,走 Dozer 发新 libs tag)。
- 不做 MCP 待办工具(S3b-2)。

## 5. 设计

### 5.1 结构

```
digger-desktop
├─ adapters/todo.rs         TodoPanel:每项目一份 WorkspaceState、服务束、结果回流通道、PanelDisplay 实现、提示排空、消息分派
├─ adapters/todo_native.rs  Native:wry 子视图(自定义协议 digger-todo://)、代数(epoch)、WebviewPushState、IPC → Event
├─ shell.rs                 "digger.todo":图标 ListTodo、配对规则(列表 | 内容,默认 Left)、默认布局
├─ product.rs               Msg::Todo / Msg::NativeTodo、todo_view、sync_native、切项目/切入面板钩子
└─ build.rs                 把库的 todo-content 资源拷进 OUT_DIR(加 todo- 前缀,避免与用量的 host.html 同名覆盖)
```

### 5.2 数据流

1. 面板可见且有当前项目 → `TodoPanel::activate(project_id)` → 库的 `on_activate` 经 `TodoService`(= `Client`)拉列表和分类树 → `Message::Loaded` / `CategoriesLoaded` 回流到 `rx` → `update`。
2. 每帧 `sync_native(window, visible, rect, &state)`:算出 `current_view_payload`,经 `WebviewPushState::pending_push` 去重,Ready 后 `evaluate_script` 推给页面;rect 变化时 `set_bounds`。
3. 页面事件(点勾选、拖动、改日期……)经 `window.ipc.postMessage` → `parse_todo_event` → `Event::Todo(event)` → 产品用 `route_event(items, event)` 映射成 `Message` / `AssignAgent` / `OpenDetail` → 进 `update`。
4. 写操作:乐观更新 → `TodoService` → 不管成败都 `Mutated` → 重新 `Loaded`(库的既有约定)。

### 5.3 关键决定

- **D1 复用 `WebviewPushState`,不新建 `TodoSurface`。** 用量库有 `UsageSurface`(走 `bytehost-desktop` 的 `SurfaceExecutor`),待办库没有,只有纯逻辑的 `WebviewPushState`(ready、去重、加载超时、失败)。Digger 直接用它,**不动库、不动 Dozer**。代价:没有 Surface 的"代数校验"(旧 webview 的在途 IPC 被拒);由 `todo_native` 自己的 `epoch` 补上(与 `usage_native` 同款:每次销毁递增 epoch,旧 epoch 的 Ready/IPC 丢弃)。如果以后想让两个产品都走 `SurfaceExecutor`,那是一次独立的 Dozer 库改动。
- **D2 每个项目一份 `WorkspaceState`,webview 只有一个。** 切项目只是 `set_desired` 换一份 payload,不重建 webview。库的 `requests`(请求代数)保证上一个项目的在途结果被丢弃。
- **D3 资源按名字隔离。** 用量与待办的入口页都叫 `host.html`;`build.rs` 拷到 `OUT_DIR` 时待办一律加 `todo-` 前缀(`todo-host.html`),`include_bytes!` 引用同名。不隔离会让后拷的覆盖先拷的,两个面板其中一个白屏。
- **D4 严格 CSP 不放宽。** 库的 `host.html` 自带 `default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'`,`digger-todo://` 协议只服务三个文件、其余 404、导航只放行本协议。
- **D5 服务束。** `PanelIo` 需要完整的 `Services`;沿用 `ProjectPanel::new` 的做法,只有 `todo` 是真实的(`Arc<Client>`),其余填 `UnavailableService`。把 `UnavailableService` 提成 `adapters` 共用(目前在 `project.rs` 里私有)。
- **D6 "指派 agent" 不走 `TodoService`。** `assign_todo_agent` 是 `Client` 的固有方法,**不在 `TodoService` trait 里**(Dozer 的 `App::todo_assign_agent` 也是直接调 `client.assign_todo_agent`,读 `dozer-app/src/app/update.rs:3096`)。所以 `TodoPanel` 要同时持有 `Arc<dyn TodoService>`(给库用)和一个 `Client`(给指派用)。不为此改库:把它补进 trait 是更干净的长期做法,但会动 Dozer 的服务适配与所有 fake,作为独立的 Dozer 改动另议,不挡本期。

## 6. 验证

- 单元:`adapters/todo.rs` 的消息分派表(每个库 `Message` 变体都有明确去向,新增变体编译失败)、按项目隔离、过期结果丢弃、提示排空;`todo_native.rs` 的 epoch 与去重、资源齐全、CSP 无外部引用、导航白名单。
- 集成:起隔离的真 `diggerd`(`examples/product_daemon` 的做法,独立 `DIGGER_DATA_DIR`),经 `Client` 增删改查,断言面板状态与库里数据一致。
- 真机:沙箱 Digger(独立数据目录和临时 HOME)逐项勾选,清单随计划附在 S3b-1 的验收任务里。
- 门禁:`scripts/release.sh` 全套(fmt / clippy `-D warnings` / 依赖门禁 / 全量测试),在不含陈旧忽略文件的干净快照里跑一遍。

## 7. 待你裁决的问题(都有默认值)

| # | 问题 | 默认 | 为什么 |
|---|---|---|---|
| Q1 | 待办面板在 Digger 默认放哪? | **左区**,在 Project / Files 之后 | 与 Dozer 一致(内容型面板放左区);配对规则同 Project / Files |
| Q2 | 待办详情(回合、回复框)要不要做? | **不做,留作 S3b-3 之后** | 需要浮层窗口设施,而 Digger 没有;且"处理"依赖 Q3。点卡片"打开详情"暂无反应,在验收清单里明确标为已知缺位 |
| Q3 | Digger 要不要开 `task_poller` / `summaries`(处理任务、自动派发)? | **不开** | S6a D7 默认关闭,它会调用外部 LLM 并产生费用,这是产品决定不是接线工作。指派 agent 在 Digger 里只记录归属,不执行 |
| Q4 | Digger 没有 Toast,待办写失败怎么让用户知道? | **先落 stderr,列表刷新回原样** | 与 Project 面板一致,不在本期为一个面板引入提示机制;如果真机验收觉得不够再单独立项 |
| Q5 | `MCP` 待办工具(S6e-3b)是否和面板一起发? | **S3b-2 单独一期,面板验收通过后再做** | 工具只是改 `digger-mcp` 入口的一个特性开关,但要面板先能被验收,否则 agent 改了数据、用户看不到面板 |

## 8. 风险

1. **webview 盖住 iced 控件。** 内容区下方有 iced 的新增输入框、左栏有清空栏,原生子视图恒在 iced 之上。几何必须同源,不能凭经验留白:用库导出的 `top_row_inner_h_px()` 与 `WorkspaceState::add_input_height()` 算,并加几何测试(S3b-1 Task 4)。
2. **库的 `Message` 变体有一批"内核拦截"项**,Digger 若漏处理会静默丢事件;用穷尽 `match` 让新增变体编译失败。
3. **`thread_local` 陷阱。** `view()` 在主线程,订阅回调在 tokio 工作线程;凡是订阅回调要读的状态一律用进程级原子,**不用 `thread_local`**(S6d 里因此丢过键盘输入)。`todo_native` 里只有 wry 视图句柄放 `thread_local`(只在 `window::run` 闭包里、即主线程访问),与 `usage_native` 同。
4. **库 tag 一致性。** 新增 `dozer-panel-todo` 依赖必须与 Digger 现有的 `libs-2026-10-24e` 同 tag,否则 `check-shared-deps.sh` 会拦(会出现两份 byteui、两份主题单例)。
