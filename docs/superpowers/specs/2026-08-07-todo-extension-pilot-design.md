# Todo 面板扩展化设计

**状态:已批准(brainstorming 会话,2026-08-07)**

## 背景

Git Log(App 级共享状态)与浏览器(per-project 状态 + 收藏夹整体划归)两个阶段 1 试点均已
设计定案(见对应 spec/plan)。Todo 面板是原始候选清单里的第三个,也是耦合最深的一个——
memory 里早有记录"派发逻辑深入终端/会话领域,耦合比浏览器更深"。这次沿用同一套"自己的
`Message`/`State`/`update`/`view`,内核包装转发"的阶段 1 模式,但 Todo 面板的状态归属和
跨领域调用比前两个试点都复杂,需要专门设计。

用户已确认排期:Todo 面板 implementation 的执行要等浏览器面板 Task 5/6 落地后再开始(两边
都会大幅改 `workspace.rs`,同时改会冲突)。这次设计/写计划本身不碰代码,不受这个排期限制。

## 目标 / 非目标

**目标**:
1. `extensions::todo` 拥有自己的 `Message`/`update`/`view`,内核只留一个包装变体
   `Message::Todo(extensions::todo::Message)` 做转发。
2. **状态拆成两块**(这是跟前两个试点最大的形状差异):
   - `todo::WorkspaceState`——挂在每个 `Workspace` 上,对应现在的 8 个 `todo_*` 字段
     (`items`/`mtime`/`add_draft`/`filter`/`search`/`dispatch_open`/`pending_dispatch`/
     `editing_plan_date`)。
   - `todo::AppState`——挂在 `App` 上,对应现在的 `todo_meta: TodoMetaState`
     (`HashMap<ProjectId, HashMap<u64, TodoTaskMeta>>`,内部仍按项目分桶,但作为单一字段
     整体挂在 `App` 而非拆成每项目一份)。
3. `.dozer/todo.md` 的读/解析/写(现在 `Workspace::toggle_todo_item`/`submit_todo_add`
   里内联的文件 I/O)搬进 `extensions::todo`,以"项目路径由内核每次调用传入"的函数形式存在
   ——模块本身不知道"现在是哪个项目"。
4. `record_todo_dispatch`/`set_todo_plan_date`/`set_todo_completed_at`(现在的 `App`
   方法,写 `todo_meta` 并落盘)原样变成 `extensions::todo` 的普通函数,内核直接调用,不
   经过 `Message`/`update` 分发——完全照抄现状"闭包内改 `Workspace`、闭包外单独调一次改
   `App` 级状态"这个已有的两步走结构,只是函数搬家。
5. `TodoDispatchToExisting`/`TodoDispatchNew` 两条消息由内核在到达 `todo::update` 之前
   拦截:真正的终端会话写入/新建终端(`ws.tabs`/`io.client.write()`)留在 `workspace.rs`
   不动,事后调 `todo::record_dispatch(..)` 记一笔派发记录。这是跟 Git Log 的 `LoadMore`、
   浏览器的两个异步结果变体同一原则的第三次应用:内核特案需要跨领域能力的消息,其余统一
   转发。
6. `todo::update` 统一接收 `WorkspaceState`/`AppState` 两块状态,处理 12 条消息里的 10 条
   (含需要读写 `AppState` 的 `TodoToggle`/`TodoPlanDateEditStart`/`TodoPlanDateSubmit`)。

**非目标**:
- 不改变终端会话派发的实际行为(写入已有 session / 新建 session 再写入文本)——这部分
  保持在内核,只是调用点从内联散在 `update()` 里变成一次跨模块函数调用。
- 不改 `.dozer/todo.md` 的文件格式/解析规则,不改 `todo_meta.json` 的 schema。
- 不建 `Extension` trait/注册表,不拆独立 crate,不引入 `iced::Task`/`Command` 风格异步。
- 不改轮询节奏/触发条件(`App::poll_todo_if_visible` 判断"现在该不该轮询"的逻辑不变)。
- implementation 不在这次设计/计划范围内启动——要等浏览器面板 Task 5/6 落地。

## 关键语义确认(brainstorming 会话定案)

- **状态两分**:`WorkspaceState`(per-project)+ `AppState`(App 级、内部按 `project_id`
  分桶)。这是因为现状本来就是这样分的(`todo_items` 等 8 个字段挂 `Workspace`,`todo_meta`
  挂 `App`),拆分只是把这两组字段各自搬进 `extensions::todo` 模块里定义的类型,不改变
  它们原来分别挂在哪个层级。
- **`todo_meta` 整体划归扩展,不留在内核**——跟收藏夹一样的判断标准:`todo_meta` 只服务
  Todo 面板(不像 `AddrEvent`/`WebviewSpec`/`PickerLaunch` 那样被其他领域共用),按"真正
  单一用途就整个并入"的原则处理。读写盘(`todo_meta::load`/`save`)逻辑随之一起搬,但
  `todo_meta.json` 仍是跨项目共享的单一文件,不拆分成每项目一份。
- **派发的跨领域调用**:内核特案 `TodoDispatchToExisting`/`TodoDispatchNew`,`todo` 模块
  从头到尾不知道 `session`/`Client` 这些概念——不是给 `todo::update` 传一个"派发能力"回调
  或 trait(会引入跟前两个试点风格不一致的抽象层),而是直接在内核 `match` 里做完终端相关
  的部分。
- **不止派发那两条消息碰 `AppState`**:写计划前重新核对了一遍现有代码(`TodoPlanDateEditStart`
  要从 `todo_meta` 读预填值,`TodoToggle`/`TodoPlanDateSubmit` 要写 `todo_meta`),所以
  `todo::update` 的签名统一接收两块状态一起处理,不是只在两条派发消息上开特例、其余 10 条
  只碰 `WorkspaceState`。
- **没有异步机制**:`.dozer/todo.md` 的读写是现状就有的同步 `std::fs` 调用(不是 spawn_blocking
  的异步任务),`todo::update` 不需要 `Client`/`handle`/`emit` 这套(比 Git Log/浏览器都
  简单的一面)。
- **`PickerLaunch` 不移动**:它是终端"新建 agent 会话"选择器共用的类型(`Message::AgentPickerSelect`
  也用),不是 Todo 专属,留在 `workspace.rs`,`todo::Message::DispatchNew` 只是引用它——
  跟 `AddrEvent`(浏览器试点)/`WebviewSpec` 同一处理方式的第三次应用。

## 架构与数据流

### 1. 状态类型

```rust
/// 挂在每个 Workspace 上的 Todo 面板状态。
pub struct WorkspaceState {
    items: Vec<TodoItem>,
    mtime: Option<std::time::SystemTime>,
    add_draft: String,
    filter: TodoFilter,
    search: String,
    dispatch_open: Option<usize>,
    pending_dispatch: HashMap<usize, String>,
    editing_plan_date: Option<(usize, String)>,
}

/// 挂在 App 上的 Todo 元数据(派发记录/计划时间/完成时间的持久化,
/// 按 project_id 分桶;内容原样对应现有 `todo_meta::TodoMetaState`)。
pub struct AppState {
    meta: TodoMetaState, // HashMap<i64, HashMap<u64, TodoTaskMeta>>,现有类型原样搬入
}
```

`TodoItem`/`TodoFilter`/`TodoTaskMeta`/`DispatchRecord`/`TodoMetaState` 等现有 `todo.rs`/
`todo_meta.rs` 里的类型定义原样搬进 `extensions::todo`(具体是一个文件还是拆
`extensions/todo/{mod,meta}.rs` 两个文件,写计划时按最终体量决定,不在设计阶段钉死——
参考 Git Log/浏览器两个试点都是单文件,若 Todo 体量明显更大再考虑拆)。

### 2. `Message`

```rust
pub enum Message {
    Toggle(usize),
    AddInputChanged(String),
    AddSubmit,
    FilterSet(TodoFilter),
    SearchChanged(String),
    DispatchOpen(usize),
    DispatchClose,
    DispatchToExisting(usize, String),   // 内核拦截,不进 update
    DispatchNew(usize, crate::workspace::PickerLaunch), // 内核拦截,不进 update
    PlanDateEditStart(usize),
    PlanDateChanged(String),
    PlanDateSubmit,
}
```

12 个变体对应现在顶层 `Message` 里的 12 个 `Todo*` 变体,去掉前缀原样搬来。

### 3. `update`

```rust
/// 处理 `DispatchToExisting`/`DispatchNew` 之外的 10 条消息。内核在到达这里
/// 之前已经拦截了那两条(见"内核侧改动"),它们传进来会 `unreachable!`
/// (同 Git Log 试点 `LoadMore` 的处理方式)。
pub fn update(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    msg: Message,
    project_id: i64,
    project_path: &Path,
)
```

- `Toggle(idx)`:读取 `ws_state.items[idx]`,算出 old/new 行文本,读盘 → `replace_todo_line`
  → 写盘(照搬现有 `toggle_todo_item` 逻辑),成功后重读 `items`;若替换前后文本一致(不是
  并发冲突),调用 `set_completed_at`(本模块内的私有辅助,更新 `app_state.meta`)。
- `AddInputChanged`/`FilterSet`/`SearchChanged`/`DispatchOpen`/`DispatchClose`/
  `PlanDateChanged`:纯 `ws_state` 字段赋值,逻辑不变。
- `AddSubmit`:读盘 → `append_todo_item` → 写盘 → 清空 `add_draft`(照搬
  `submit_todo_add`)。
- `PlanDateEditStart(idx)`:从 `app_state.meta` 按 `todo_line_key(&items[idx].text)` 查
  预填的 `plan_date`(没有则空字符串),置 `ws_state.editing_plan_date = Some((idx, prefill))`。
- `PlanDateSubmit`:取 `ws_state.editing_plan_date` 的草稿,写进 `app_state.meta` 对应项的
  `plan_date`,落盘,清空 `editing_plan_date`。
- `DispatchToExisting`/`DispatchNew`:`unreachable!("由内核拦截处理")`。

### 4. 内核直调的普通函数(不经过 `update`)

```rust
/// 内核在完成终端会话写入/新建之后调用,记一笔派发记录并落盘。
pub fn record_dispatch(app_state: &mut AppState, project_id: i64, text: &str, session_id: String);
```

（现有 `App::record_todo_dispatch` 的搬家版本,逻辑不变。）

### 5. 文件 I/O 与轮询

```rust
/// 重读 `.dozer/todo.md`,刷新 `items`/`mtime`。文件不存在/读失败按空列表
/// 处理,不 panic。现有 `Workspace::reload_todo_from_disk` 的搬家版本。
pub fn reload_from_disk(ws_state: &mut WorkspaceState, project_path: &Path);
```

`App::poll_todo_if_visible`(判断"当前是否正看着某项目的 Todo 面板,该不该 `stat` 一下
mtime")留在 `workspace.rs` 不动,内部改成调用 `todo::reload_from_disk`。

### 6. `view`

```rust
pub fn view(
    app_state: &AppState,
    ws_state: &WorkspaceState,
    project_id: i64,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>
```

现有 `todo_pane`/`todo_row`/`todo_filter_segment`/`app_todo_dispatch_for` 等渲染函数整体
搬过来,签名从吃 `app: &App, ws: &Workspace` 改吃 `app_state: &AppState, ws_state: &WorkspaceState`,
`Message` 类型从顶层换成本模块的。

### 7. 内核侧(`workspace.rs`)改动

`App` 新增字段 `todo: todo::AppState`(取代 `todo_meta: TodoMetaState`)。

`Workspace` 上 8 个 `todo_*` 字段合并成 `todo: todo::WorkspaceState`。

顶层 `Message` 删除 12 个 `Todo*` 变体,加 `Todo(todo::Message)`。

`update()` 分三支:
- `Message::Todo(todo::Message::DispatchToExisting(idx, session_id))`:取文本(同现有
  `self.active_workspace().and_then(|ws| ws.todo_items.get(idx))` 的读法)、
  `with_focused_project` 内做 `ws.todo.dispatch_open = None` + 终端写入(现有
  `dispatch_todo_to_existing` 不动,继续是 `Workspace` 方法),闭包外调用
  `todo::record_dispatch(&mut self.todo, project_id, &text, session_id)`。
- `Message::Todo(todo::Message::DispatchNew(idx, launch))`:同理,新建终端部分照现有
  `spawn_new_tab` 路径不动,事后一样调 `record_dispatch`。
- `Message::Todo(msg)` 兜底:`with_focused_project` 内取 `project_id`/`project_path`,
  调用 `todo::update(&mut ws.todo, &mut self.todo, msg, project_id, &project_path)`——
  这里需要注意 `self.todo`(App 级)和 `ws`(某个 `Workspace`)的可变借用要能同时成立,写
  计划时按 `with_focused_project`/`with_project` 现有的借用分离方式(`io: &ShellIo` 已经
  是从 `self` 拆出来的独立借用)处理,必要时把 `self.todo` 的可变引用在调用
  `with_focused_project` 之前单独取出。

`App::view()` 的 `LeftView::Todo` 分支:`todo::view(&app.todo, &ws.todo, project_id)
.map(Message::Todo)`。

## 错误处理

不新增错误处理路径,原样保留:
- 文件读取失败 → 空列表,不 panic。
- 写盘冲突(`replace_todo_line` 返回 `None`,说明文件已被并发改过)→ 静默放弃这次写入,
  下次轮询自然重读最新内容。
- 派发目标 session 已不存在/已退出 → 现有降级路径(`tracing::warn!` + no-op)不变,这部分
  逻辑本来就在内核(`dispatch_todo_to_existing`),不受这次拆分影响。

## 测试策略

- `extensions::todo` 里的纯逻辑(`parse_todo`/`replace_todo_line`/`append_todo_item`/
  `filter_todos`/`todo_line_key`,现有 `todo.rs`/`todo_meta.rs` 已有的测试)原样保留,只
  搬文件位置。
- 新增 `update`(10 条消息的状态转换,含 `AppState` 读写)、`record_dispatch`、
  `reload_from_disk` 的单测,镜像 Git Log/浏览器两个试点的写法。
- 人工验收:勾选完成(含 `completed_at` 打时间戳)、新增任务、筛选/搜索、派发到已有会话、
  派发新建会话(记录落 `todo_meta.json`)、计划时间编辑(含预填)、轮询在面板打开时自动
  刷新——对照 Todo 面板现有验收清单走一遍,确认拆分没有改变任何可见行为。
- 编译期防回归:`cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p
  dozer-app --all-targets && cargo fmt --check` 全绿。

## 依赖变更

无新增依赖。
