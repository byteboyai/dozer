# Usage(用量面板)面板扩展化设计

**状态:已批准(brainstorming 会话,2026-08-07)**

## 背景

Git Log(App 级共享状态)、浏览器(per-project 状态)、Todo(状态两块 + 派发逻辑跨终端/
会话领域)、Files(状态两块,多个带副作用的异步文件系统操作)四个阶段 1 试点均已完工并
接入内核,只剩各自的人工 GUI 验收未走。这次评估下一个候选时点检了 `workspace.rs`(现
9307 行)里还没套上阶段 1 模式的两块:Conversations(挖开 `ConversationOpen` 才发现它
实际是"对话列表 + 审阅内容"配对面板,4 条消息且要读 `ws.tabs` 判断会话来源,耦合深度接近
Todo,值得单独走一轮完整 brainstorming)、Usage(`usage.rs` 已经几乎全是纯函数,状态只有
`usage`/`usage_loading` 两个纯 per-project 字段,消息只有 `UsageRefresh`/`UsageLoaded`
两条)。选 Usage 是因为它是五个试点里形态最简单的一个——第一个"完全不需要 `AppState`"
的试点,验证阶段 1 模式在这种最小形态下依然适用、套壳成本是否真的像预期一样低。

沿用同一套"自己的 `Message`/`WorkspaceState`/`update`/`view`,内核包装转发"的阶段 1
模式。

## 目标 / 非目标

**目标**:
1. 新建 `extensions::usage`,拥有自己的 `Message`(`Refresh`/`Loaded`)、`WorkspaceState`
   (`rows: Vec<(ConversationMeta, ConversationUsage)>`/`loading: bool`,对应现有
   `Workspace` 上 `usage`/`usage_loading` 两个字段)、`update`、`spawn_refresh`(自由
   函数,替代现有 `Workspace::spawn_usage_refresh`)。
2. `usage.rs` 现有全部纯函数(`parse_usage`/`parse_claude_shaped_usage`/
   `parse_codebuddy_shaped_usage`/`aggregate`/`group_usage_by_agent`/
   `daily_totals_by_agent`/`agent_token_share`/`view`/`panel_header`/`summary_card`/
   `usage_row`/`grouped_list`/`bar_segment`/`bar_chart`/`format_token_short`/
   `PieChart`/`pie_chart`/`chart_legend`,以及它们现有的全部单测)整体搬进
   `extensions/usage.rs`,逻辑不变,只把 `use crate::workspace::Message` 换成模块自己的
   `Message`,`impl canvas::Program<Message, ..> for PieChart` 的 `Message` 同步换成
   `usage::Message`。
3. 内核 `RightIconSelect(RightView::Usage)` 分支(现有:切到 Usage 视图时触发刷新,这条
   消息四个 `RightView` 共用、不专属 Usage)改成直调 `usage::spawn_refresh`,不下放整条
   消息——同 Files 试点 `ProjectFsChanged` 触发 `files::spawn_git_refresh` 的处理方式。
4. `Message::Usage(usage::Message::Refresh)`(手动点刷新按钮)转发进 `usage::update`,
   内部同样调用 `spawn_refresh`——两个触发入口共用一份异步逻辑,同 Git Log 试点
   `LoadMore`/引用变化触发共用 `request_refresh` 的打法。
5. `Loaded` 是异步结果,必须按自带的 `ProjectId` 走 `with_project` 路由(不是
   `with_focused_project`)——同 Files/Todo/浏览器三个试点已经验证过的既有不变式。

**非目标**:
- 不改变任何用户可见行为:统计中占位/空态文案、汇总卡片、柱状图/饼图渲染、"只在切到
  面板或点刷新按钮时才重新扫"的节流规则,一律原样保留。
- 不建 `Extension` trait/注册表,不拆独立 crate。
- 不新增 `AppState`——Usage 的两个字段都是纯 per-project 数据,不涉及跨项目共享的浮层/
  元数据/网络客户端,是五个试点里第一个"只有 `WorkspaceState`"的形态。
- 不处理 Conversations 面板——它跟 Usage 同挂 `RightView` 但彼此独立,耦合更深(4 条
  消息 + 读 `ws.tabs` 判断会话来源),排期上留到以后单独一轮 brainstorming。

## 关键语义确认(brainstorming 会话定案)

- **状态形态是五个试点里最简单的一种,但异步结果仍要按 `project_id` 路由**:没有
  `AppState` 不代表不需要区分路由方式——`Loaded(project_id, rows)` 落地时用
  `with_project(project_id, ..)`,`Refresh`(用户点按钮,发生在当前聚焦项目上)走
  `with_focused_project`。这条不变式(异步结果按消息自带的 `project_id` 投递,不看"此刻
  聚焦的是谁")跟状态是否拆分成两块无关,是所有异步结果消息通用的规则。
- **`spawn_refresh` 两个触发入口共用同一份实现**:`RightIconSelect(Usage)`(切到面板时
  自动刷新)与 `Message::Usage(Refresh)`(手动点刷新按钮)现有代码就已经调同一个
  `Workspace::spawn_usage_refresh`,迁移后延续这个结构——前者内核直调
  `usage::spawn_refresh`,后者经 `usage::update` 间接调用,两条路径不重复实现异步扫描
  逻辑。
- **`WorkspaceState` 不持有 `project_id`/`project_path`**:内核在调用 `update`/
  `spawn_refresh` 时显式传入,模块本身不记"现在是哪个项目"——同 Todo/Files 试点的处理
  原则(状态归属跟着数据本身走,项目身份是内核概念)。
- **`PieChart` 泛型化不是必须的**:它的 `draw` 从不构造 `Message`,直接把
  `impl canvas::Program<Message, ..>` 里的 `Message` 换成 `usage::Message` 即可编译,
  不需要像 `crate::workspace::status_bar_container`(Files 试点)那样做成
  `<Msg: 'a>` 泛型——`PieChart` 只在 `usage::view` 内部被构造和使用,没有跨模块复用的
  需求。

## 架构与数据流

### 1. 状态类型

```rust
/// 挂在每个 Workspace 上的 Usage 面板状态,对应现有 `usage`/`usage_loading`
/// 两个字段。
#[derive(Default)]
pub struct WorkspaceState {
    rows: Vec<(ConversationMeta, ConversationUsage)>,
    loading: bool,
}

impl WorkspaceState {
    pub fn rows(&self) -> &[(ConversationMeta, ConversationUsage)] {
        &self.rows
    }

    pub fn loading(&self) -> bool {
        self.loading
    }

    /// 供内核 `RightIconSelect(RightView::Usage)` 分支调用——切到面板时
    /// 立即标记"统计中",不等 `spawn_refresh` 的异步结果落地才置真,否则
    /// 用户会先看到旧数据/空态闪一下再变成"统计中"。
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }
}
```

### 2. `Message`

```rust
pub enum Message {
    Refresh,
    Loaded(i64, Vec<(ConversationMeta, ConversationUsage)>),
}
```

对应现在顶层 `Message` 里的 `UsageRefresh`/`UsageLoaded` 两个变体,去前缀原样搬来。

### 3. `update`

```rust
/// `project_path` 由内核传入(`ws.project.as_ref().map(|p| p.path.clone())`),
/// 不需要 `Option` 包装——调用方(内核)已经在 `with_focused_project`/
/// `with_project` 闭包里确认过 `ws.project.is_some()` 才会走到这里
/// (`RightIconSelect(Usage)`/`Message::Usage(Refresh)` 两个触发点都是如此,
/// 没有项目时这两条路径本来就不会被触发——写计划时如果发现有反例,改成
/// `Option<PathBuf>` 并在 `None` 时提前 `return`)。
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    project_path: PathBuf,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::Refresh => {
            ws_state.loading = true;
            spawn_refresh(project_id, project_path, handle, emit);
        }
        Message::Loaded(_, rows) => {
            ws_state.rows = rows;
            ws_state.loading = false;
        }
    }
}
```

### 4. 内核直调的自由函数

```rust
/// 异步扫描项目全部 agent transcript 并逐个解析用量。内核在
/// `RightIconSelect(RightView::Usage)` 分支(切到面板首次刷新)与
/// `usage::update` 处理 `Refresh`(手动点刷新按钮)两处调用。现有
/// `Workspace::spawn_usage_refresh` 的搬家版本,逻辑不变(读失败的会话
/// 整条跳过、不计入汇总)。
pub fn spawn_refresh(
    project_id: i64,
    project_path: PathBuf,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let rows = tokio::task::spawn_blocking(move || {
            crate::conversation::list_all_conversations(&project_path)
                .into_iter()
                .filter_map(|meta| {
                    let jsonl = std::fs::read_to_string(&meta.path).ok()?;
                    let u = parse_usage(meta.agent, &jsonl);
                    Some((meta, u))
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        emit(Message::Loaded(project_id, rows));
    });
}
```

### 5. `view`

```rust
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    project_name: &'a str,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>
```

现有 `view` 签名的 `rows: &[(..)], loading: bool` 两个参数合并成 `ws_state:
&WorkspaceState`(内部改读 `ws_state.rows()`/`ws_state.loading()`),函数体逻辑不变。
`panel_header`/`summary_card`/`usage_row`/`grouped_list`/`bar_segment`/`bar_chart`/
`pie_chart`/`chart_legend` 等私有辅助函数原样搬入,`Message` 类型从顶层换成本模块的。

### 6. 内核侧(`workspace.rs`)改动

`Workspace` 上 `usage: Vec<...>`/`usage_loading: bool` 两个字段合并成 `usage:
usage::WorkspaceState`(`#[derive(Default)]`,合并进 `empty_for_project_placeholder()`
的初始化)。`adopt_project()` 里现有 `self.usage = Vec::new(); self.usage_loading =
false;` 两行改成 `self.usage = usage::WorkspaceState::default();`。

顶层 `Message` 删除 `UsageRefresh`/`UsageLoaded` 两个变体,加 `Usage(usage::Message)`。

`RightIconSelect(RightView::Usage)` 分支里:

```rust
self.with_focused_project(|ws, io| {
    let Some(project) = &ws.project else { return };
    let project_id = project.id;
    let project_path = PathBuf::from(&project.path);
    ws.usage.set_loading(true);
    let proxy = io.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::Usage(m));
    };
    usage::spawn_refresh(project_id, project_path, &io.handle, emit);
});
```

（`project_id`/`project_path` 从 `ws.project` 取,`emit` 构造 `Message::Usage(m)`
经 `proxy.send_event` 发回,写法同 Files 试点 `spawn_git_refresh` 调用点。）

`update()` 里 `Message::Usage` 分两支路由(同 Files 试点"异步结果按 `project_id`,其余
按聚焦"的三支模式,Usage 少一层——不需要 `loaded_workspace_mut` 借用分离技巧,因为没有
`AppState` 要同时借用):

```rust
Message::Usage(msg @ usage::Message::Loaded(project_id, ..)) => {
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    let emit = move |m| { let _ = proxy.send_event(Message::Usage(m)); };
    self.with_project(project_id, move |ws, _io| {
        let project_path = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
        if let Some(project_path) = project_path {
            usage::update(&mut ws.usage, msg, project_id, project_path, &handle, emit);
        }
    });
}
Message::Usage(msg) => {
    let Some(project_id) = self.active_project_id else { return };
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    let emit = move |m| { let _ = proxy.send_event(Message::Usage(m)); };
    self.with_focused_project(move |ws, _io| {
        let Some(project_path) = ws.project.as_ref().map(|p| PathBuf::from(&p.path)) else {
            return;
        };
        usage::update(&mut ws.usage, msg, project_id, project_path, &handle, emit);
    });
}
```

两支不能合并,这不是留给写计划阶段的待定项——`Refresh` 结构上不带 `project_id`(语义
就是"当前聚焦项目",天然只能用 `with_focused_project` 解析出 `project_id`),`Loaded`
结构上带 `project_id` 且必须按它路由(哪怕已经切走了别的项目,结果也要投回发起它的
项目)。两者路由方式不同是消息形状决定的,不是实现细节,写计划时原样保留这个两支结构。
`Loaded` 分支里 `usage::update` 内部不会真的调用 `spawn_refresh`(`Message::Loaded`
只匹配 `update` 里的落地分支),传 `project_path` 给它只是保持 `update` 单一签名、两条
消息共用,不是有额外用途。

`App::view()` 的 `RightView::Usage` 分支,现有调用:

```rust
RightView::Usage => usage::view(
    &ws.usage,
    ws.usage_loading,
    ws.project.as_ref().map(|p| p.name.as_str()).unwrap_or("未打开项目"),
    Length::Fill,
    zone_pane_border(zone, ac),
),
```

改成 `usage::view(&ws.usage, ws.project.as_ref().map(|p| p.name.as_str())
.unwrap_or("未打开项目"), Length::Fill, zone_pane_border(zone, ac))`——去掉现在单独传的
`ws.usage_loading`(并入 `ws.usage` 这个 `WorkspaceState`),`project_name` 的
`unwrap_or("未打开项目")` 兜底原样保留(没有项目时也要能画出面板,不是 `Option` 提前
返回空)。

## 错误处理

不新增错误处理路径,原样保留:
- 读某个 transcript 文件失败(权限/IO error)→ 该会话整条跳过,不计入汇总,不影响其余
  会话的统计(现有 `spawn_usage_refresh` 行为)。
- 解析失败/字段缺失 → 该行跳过或记 0,不 panic(现有 `parse_usage` 行为)。

## 测试策略

- `parse_usage`/`aggregate`/`group_usage_by_agent`/`daily_totals_by_agent`/
  `agent_token_share`/`format_token_short` 等纯函数的现有测试原样保留,只搬文件位置。
- 新增 `update` 的单测:`Refresh` 设置 `loading = true`,`Loaded` 落地清 `loading`
  并写入 `rows`。
- 人工验收:切到用量面板触发首次扫描(显示"统计中…")、点手动刷新按钮、空态文案
  (无对话记录时)、切换项目页签各自统计独立、柱状图/饼图/分组列表渲染正常。
- 编译期防回归:`cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p
  dozer-app --all-targets && cargo fmt --check` 全绿。

## 依赖变更

无新增依赖。
