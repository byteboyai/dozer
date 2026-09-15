# Conversations(对话面板)扩展化设计

**状态:已批准(brainstorming 会话,2026-09-15)**

## 背景

[[dozer-extension-architecture-idea]] 记录的阶段 1 试点(Files/GitLog/Todo/Project/
Database/Ssh/Web/Usage,共 8 个)已全部完工并接入内核,`workspace.rs` 从 11000+ 行降到
4611 行。当时评估时 Conversations 因"耦合深,值得单独 brainstorming"被搁置,此后
`workspace.rs` 又叠加了 webview 化(`dozer://review-trace`)、trace 详情"加载更多"、
客户端分页、agent 筛选下拉等功能,耦合比当初评估时更深。这次实地摸底代码后确认:
真正深的耦合只有一处(见下),其余部分形态跟已完工的试点相当。

`PanelKind` 10 个 rail 面板里,`Agent`(终端 tab + tab 列表)是核心面板,按既定原则
不在剥离范围;`Conversations` 是唯一还没套阶段 1 模式的面板。

## 目标 / 非目标

**目标**:
1. 新建 `extensions::conversations`(模块名用复数,理由见"关键语义确认"),拥有自己的
   `Message`、`WorkspaceState`、`update`、`spawn_refresh`、`view`(仅列表侧)。
2. `Workspace` 上 7 个平铺字段(`conversation_sessions`/`conversation_pages`/
   `conversation_search`/`conversation_search_draft`/`conversation_search_focused`/
   `conversation_agent_filter`/`conversation_agent_picker_open`)合并成
   `conversations: conversations::WorkspaceState`。
3. 顶层 `Message` 里 9 个平铺的 `ConversationXxx` 变体收拢成
   `Conversations(conversations::Message)`。
4. 会话列表面板(搜索框/agent 筛选下拉/分页/列表卡片,现
   `conversation_list_pane`/`conversation_footer_bar`/`conversation_agent_picker_view`/
   `filter_sessions`/`conversation_agents_present`/`conversation_visible_count`/
   `conversation_search_field_id`/`CaptureConversationSearchFocus`/
   `take_conversation_search_focused`,`workspace.rs` 约 2506-2910 区间)整体搬进
   `extensions/conversations.rs`。
5. `spawn_conversations_refresh` 搬成自由函数 `conversations::spawn_refresh`,
   `Workspace` 保留同名薄封装委托(同 Files/Usage 试点做法)。

**非目标**:
- 不改变任何用户可见行为:搜索/筛选/分页/"当前会话"高亮/关联任务后缀等一律原样保留。
- **详情审阅区(`review_content`/`load_more_button`/`ReviewView`/`ReviewSource`/
  `review_webview_spec`/webview host 机制)不搬**——这是本次和之前 8 个试点最大的
  不同点,原因见下一节。这部分继续留在 `workspace.rs`。
- `ConversationSessionOpen`/`ConversationDetailLoadMore` 两个变体的**处理逻辑**不搬进
  extension(见下一节),只是包装进 `conversations::Message` 统一入口。
- 不新建 `Extension` trait/注册表,不拆独立 crate,不引入类型擦除。
- 首页 `home_recent_conversations`(dozer-home-tab"最近对话"卡片)不在本次范围——用户
  已明确只处理项目内 rail 上的 Conversations 面板。

## 关键语义确认(brainstorming 会话定案)

- **面板"一分为二":列表归 extension,详情审阅区留内核。** `ws.review: Option<ReviewView>`
  是内核共享槽位,`ReviewSource` 只有 `Session`(终端 tab 审阅,核心 `Agent` 面板专属)和
  `Conversation`(本面板)两种,渲染统一走 `dozer://review-trace` webview host——这是服务
  两种 source 的内核基础设施,不是 Conversations 私有的。沿用 Project 试点定下的原则
  ("多消费方共享数据,组合计算权收归内核"):`ReviewView`/`ReviewSource`/`ReviewEntry`/
  `review_webview_spec`/`spawn_review_load_conversation` 全部留在 `workspace.rs`,
  extension 完全不知道 `ws.review` 的存在。
- **`SessionOpen`/`DetailLoadMore` 包装进 extension 的 `Message`,但内核拦截处理。**
  为了让顶层 `Message` 只留一个 `Conversations(..)` 包装变体(不再有平铺的
  `ConversationSessionOpen`/`ConversationDetailLoadMore`),这两个变体作为
  `conversations::Message` 的成员存在,但内核 `update()` 在 `Message::Conversations(msg)`
  分支顶部用 `@` 绑定先特判这两种(同 Usage 试点
  `Message::Usage(msg @ usage::Message::Loaded(..))` 的既有拦截写法),原样保留现有
  `conversation_session_open`/`Message::ConversationDetailLoadMore` 分支的处理逻辑,
  不转发进 `conversations::update`。`conversations::update` 自身永远不会收到这两个变体
  (拦截优先于转发,见架构小节内核 `update()` 四支分派顺序),但 `match` 仍需覆盖它们才能
  编译——写成 `unreachable!(..)`,并在 `conversations::Message` 的文档注释注明
  "`SessionOpen`/`DetailLoadMore` 由内核 `App::update` 直接处理,不会到达这里"。
- **`conversation_session_open` 需要读 extension 状态,通过只读访问器。** 该方法要从
  `ws.conversation_sessions` 找到对应行取 `summary_title`/`summary_text`/`last_ts`
  ——迁移后改成 `ws.conversations.sessions()`(新增只读访问器,替代直接字段访问)。
- **模块命名用复数 `conversations`,不用 `conversation`。** `crate::conversation`
  已经是一个独立的顶层数据层模块(`ConversationMeta`/`SessionRow`/
  `is_current_conversation_id`,给 Usage 面板也用),如果新 extension 模块叫
  `conversation` 会导致 `extensions::conversation` 与 `crate::conversation` 名字
  高度相似、且模块内部还要 `use crate::conversation::{SessionRow, ...}`,极易读混。
  `PanelKind::Conversations` 本身是复数,直接用 `extensions::conversations` 命名,
  自然区分,不需要额外别名。
- **跨 extension 只读——列表卡片的"关联任务"后缀读 `todo` 状态。** 现有
  `conversation_list_pane` 里 `task_suffix` 直接 `ws.todo.items().iter().find(...)`。
  迁移后 `conversations::view` 增加一个 `todo_items: &[dozer_core::protocol::TodoInfo]`
  参数,由内核
  调用时传入 `ws.todo.items()`——这不是 extension 间直接耦合(extension 本身不持有
  `todo::WorkspaceState`,不知道它的存在),只是内核在组装 `view` 调用时跨读两份状态,
  同 Project 试点"内核持有共享数据"的既有原则。`open_transcript_paths()`("当前会话"
  高亮判断)同理,作为 `&[String]` 参数传入。
- **异步结果路由不变式**:`SessionsRefreshed(ProjectId, Result<...>)` 按消息自带的
  `project_id` 走 `with_project`;`spawn_refresh` 的触发(切面板/搜索/筛选/分页)走
  `with_focused_project`——与 Files/Todo/Usage/浏览器四个试点已验证过的既有规则一致。

## 架构与数据流

### 1. 状态类型

```rust
// crates/dozer-app/src/extensions/conversations.rs
use crate::app::ProjectId;
use crate::conversation::SessionRow;
use dozer_core::protocol::AgentKind;

#[derive(Default)]
pub struct WorkspaceState {
    sessions: Option<Vec<SessionRow>>,
    pages: usize,
    search: String,
    search_draft: String,
    search_focused: bool,
    agent_filter: Option<AgentKind>,
    agent_picker_open: bool,
}

impl WorkspaceState {
    pub fn sessions(&self) -> Option<&[SessionRow]> {
        self.sessions.as_deref()
    }
    pub fn search_focused(&self) -> bool {
        self.search_focused
    }
    pub fn set_search_focused(&mut self, focused: bool) {
        self.search_focused = focused;
    }
    // 其余字段供 `view`/`update` 内部使用,不必全部 pub——按写计划阶段实际
    // 调用点决定可见性,原则是"只暴露内核真正需要跨模块读的"。
}
```

`App::conversation_search_focused()`/`set_conversation_search_focused()`(现挂在
`App` 上,给 main.rs 每帧焦点捕获用)委托改读写 `ws.conversations.search_focused()`/
`set_search_focused()`——路由方式不变,只是最终落点换了个字段拥有者。

### 2. `Message`

```rust
#[derive(Debug, Clone)]
pub enum Message {
    SessionsRefreshed(ProjectId, Result<Vec<SessionRow>, String>),
    /// 由内核 `App::update` 直接拦截处理(见"关键语义确认"),不会到达
    /// `conversations::update`。
    SessionOpen(String, AgentKind),
    /// 同上。
    DetailLoadMore(String, i64),
    ListMore,
    SearchInput(String),
    SearchSubmit,
    AgentFilterSelect(Option<AgentKind>),
    AgentPickerOpen,
    AgentPickerClose,
}
```

去 `Conversation` 前缀原样搬运现有 9 个变体的字段形状。

### 3. `update`

```rust
pub fn update(ws_state: &mut WorkspaceState, msg: Message) {
    match msg {
        Message::SessionsRefreshed(_, result) => {
            ws_state.sessions = Some(match result {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!(error = %e, "对话会话列表查询失败");
                    Vec::new()
                }
            });
        }
        Message::ListMore => ws_state.pages += 1,
        Message::SearchInput(s) => ws_state.search_draft = s,
        Message::SearchSubmit => {
            ws_state.search = ws_state.search_draft.clone();
            ws_state.pages = 0;
        }
        Message::AgentFilterSelect(agent) => {
            ws_state.agent_filter = agent;
            ws_state.pages = 0;
            ws_state.agent_picker_open = false;
        }
        Message::AgentPickerOpen => ws_state.agent_picker_open = true,
        Message::AgentPickerClose => ws_state.agent_picker_open = false,
        Message::SessionOpen(..) | Message::DetailLoadMore(..) => {
            unreachable!("由内核 App::update 直接拦截处理,不会转发到这里")
        }
    }
}
```

`SessionsRefreshed` 的 `ProjectId` 参数在这里不使用(路由已经由内核
`with_project(project_id, ..)` 完成,进到这里时已经是"投给正确项目"之后),保留在
签名里只是不改变消息形状。

### 4. 内核直调的自由函数

```rust
/// 加载(或刷新)当前项目的会话列表。签名/实现照搬现有
/// `Workspace::spawn_conversations_refresh`,`client` 参数的传入方式对齐
/// Usage 试点的既有 `spawn_refresh`(`client: &dozer_client::Client`,内部
/// `clone()` 一份再 move 进 `async move` 块)。
pub fn spawn_refresh(
    project_id: i64,
    cwd: PathBuf,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let result = client
            .list_conversations_with_summaries(&cwd.to_string_lossy(), None, 500, 0)
            .await
            .map(|rows| rows.iter().map(|(c, s)| SessionRow::from_row(c, s.as_ref())).collect())
            .map_err(|e| e.to_string());
        emit(Message::SessionsRefreshed(project_id, result));
    });
}
```

### 5. `view`(仅列表侧)

```rust
pub fn view<'a>(
    app: &'a App,
    ws_state: &'a WorkspaceState,
    todo_items: &'a [dozer_core::protocol::TodoInfo],
    open_transcript_paths: &'a [String],
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>
```

`conversation_list_pane`/`conversation_footer_bar`/`conversation_agent_picker_view`/
`filter_sessions`/`conversation_agents_present`/`conversation_visible_count` 原样
搬入(去 `conversation_` 前缀内部保留即可,不强求全部改名),内部所有
`ws.conversation_xxx` 字段访问改读 `ws_state` 对应字段,所有 `Message::ConversationXxx`
构造改成 `Message::Xxx`(外层由内核 `.map(Message::Conversations)` 包一层,同其余
8 个试点的既有 `view` 返回值 `.map()` 手法)。

`conversation_search_field_id`/`CaptureConversationSearchFocus`/
`take_conversation_search_focused` 三个函数原样搬入(纯函数,不涉及状态归属变化),
`main.rs` 里调用点的路径前缀从 `workspace::` 换成 `extensions::conversations::`。

### 6. 内核侧(`workspace.rs`/`app.rs`)改动

- `Workspace` 上 7 个平铺字段合并成 `conversations: conversations::WorkspaceState`
  (`#[derive(Default)]`),`adopt_project()` 里 7 行重置代码合并成
  `self.conversations = conversations::WorkspaceState::default();`。
- 顶层 `Message` 删除 9 个 `ConversationXxx` 变体,加
  `Conversations(conversations::Message)`。
- `update()` 里新增拦截分支(替换现有 `Message::ConversationSessionOpen`/
  `Message::ConversationDetailLoadMore` 两个分支,逻辑原样保留):

  ```rust
  Message::Conversations(conversations::Message::SessionOpen(conversation_id, agent)) => {
      self.conversation_session_open(conversation_id, agent);
  }
  Message::Conversations(conversations::Message::DetailLoadMore(conversation_id, after_turn_index)) => {
      self.with_focused_project(|ws, io| {
          ws.spawn_review_load_conversation(io, conversation_id, after_turn_index,
              CONVERSATION_DETAIL_PAGE_SIZE, true);
      });
  }
  Message::Conversations(msg @ conversations::Message::SessionsRefreshed(project_id, ..)) => {
      self.with_project(project_id, move |ws, _io| {
          conversations::update(&mut ws.conversations, msg);
      });
  }
  Message::Conversations(msg) => {
      self.with_focused_project(|ws, _io| {
          conversations::update(&mut ws.conversations, msg);
      });
  }
  ```

  四支顺序有意义:前两支(内核专属)必须排在通配的 `Message::Conversations(msg)` 之前,
  否则会被后者提前吃掉——Rust `match` 按顺序取第一个匹配分支,这不是待定项。
- `conversation_session_open` 方法体里 `ws.conversation_sessions.as_ref()...` 改成
  `ws.conversations.sessions()`。
- `PanelKind::Conversations` 的三处既有调用点(`panel_select` 切入时触发刷新 / view
  渲染时的 `conversation_list_pane(app, ws, ..)` 调用 / `list_collapsed`)分别改成
  调用 `conversations::spawn_refresh`、`conversations::view(app, &ws.conversations,
  ws.todo.items(), &ws.open_transcript_paths(), ..).map(Message::Conversations)`、
  字段路径不变(`app.dims.conversations_list_collapsed`/`conversations_split`
  这两个 App 级字段维持现状,不搬——和其余 7 个两栏面板的既有折叠/分栏机制一致)。
- `review_content`/`load_more_button` 不改动(继续读 `ws.review`),只是
  `load_more_button` 里 `Message::ConversationDetailLoadMore(..)` 改成
  `Message::Conversations(conversations::Message::DetailLoadMore(..))`。
- `conversation_list_pane` 里原有的 `Message::ConversationSessionOpen(..)`
  构造同理改成 `Message::Conversations(conversations::Message::SessionOpen(..))`。

## 错误处理

不新增错误处理路径,原样保留:
- 会话列表查询失败 → 记警告日志,列表落地为空 vec(不是保留 `None` 一直转圈),与现有
  `Message::ConversationSessionsRefreshed` 分支行为一致。
- 点开一条已经从列表消失的行(刷新与点击之间的竞态)→ 静默不打开详情,不 panic、不弹窗
  (现有 `conversation_session_open` 行为,原样保留)。

## 测试策略

- `filter_sessions`/`conversation_agents_present`/`conversation_visible_count` 等
  纯函数的现有测试原样保留,只搬文件位置,断言里的 `Message::ConversationXxx` 同步改成
  `conversations::Message::Xxx`。
- 新增 `update` 单测:`SearchSubmit` 落草稿并清零分页、`AgentFilterSelect` 清零分页并收起
  picker、`SessionsRefreshed(Err)` 落地为空 vec 而非 `None`。
- 人工验收:切到对话面板触发列表刷新、搜索框输入+回车过滤、agent 筛选下拉切换、
  "更多…"翻页、点一条会话打开详情审阅(含"加载更多"回合)、切换项目页签各自列表/搜索/
  筛选状态独立、`main.rs` 键盘路由在搜索框聚焦时仍能正常打字。
- 编译期防回归:`cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p
  dozer-app --all-targets && cargo fmt --check` 全绿。

## 依赖变更

无新增依赖。
