# Conversations(对话面板)扩展化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `workspace.rs` 里内嵌的对话(Conversations)面板列表侧(搜索/agent 筛选/分页/卡片列表)拆成自洽模块 `extensions::conversations`(自己的 `Message`/`WorkspaceState`/`update`/`spawn_refresh`/`view`),内核只留包装转发——阶段 1 扩展化重构的第九个试点。审阅详情侧(`ReviewView`/`dozer://review-trace` webview host)因为要跟核心 Agent 面板共用,继续留在内核。

**Architecture:** 新文件 `crates/dozer-app/src/extensions/conversations.rs`(模块名用复数,与已有的顶层数据层模块 `crate::conversation` 区分)。`WorkspaceState`(挂 `Workspace.conversations`,7 个字段收拢)、`Message`(11 个变体,其中 `SessionOpen`/`DetailLoadMore`/`Hover`/`TextInputMenuOpen` 四个由内核 `App::update` 直接拦截处理,不会到达 `conversations::update`)、`update`(处理其余 7 个变体的纯状态转移)、`spawn_refresh`(自由函数,替代 `Workspace::spawn_conversations_refresh` 的内部实现,该方法保留为薄封装)、`view`(仅列表侧,签名 `view(app, ws_state, todo_items, open_transcript_paths, width, outer)`)。`ReviewView`/`ReviewSource`/`review_content`/`review_content_pane`/`review_webview_spec`/`spawn_review_load_conversation` 全部不动,继续留在 `workspace.rs`。

**Tech Stack:** Rust workspace;iced 0.14;`tokio::runtime::Handle` + `emit: impl Fn(Message) + Send + 'static` 回调风格(同前 8 个试点)。

**Spec:** `docs/superpowers/specs/2026-09-15-conversations-extension-pilot-design.md`(有疑问以它为准;下面 Task 2/4 补齐了设计文档遗漏的 4 个消息变体——`Hover`/`TextInputMenuOpen` 及其内核拦截写法,设计文档只写了 `SessionOpen`/`DetailLoadMore` 两个,写计划阶段读实际代码时发现搜索框还牵扯这两条既有的通用消息,同其余 4 个试点已经踩过的既有模式,不是新决策)。

## Global Constraints

- **在独立分支 `feature/conversations-extension-pilot` 上开发,不要直接提交到 main**;全部 Task 完成、构建/测试/clippy/fmt 全绿后提请审阅,审阅通过再合并——同上一轮反馈记录,避免和其他并行在制品在同一条分支上互相干扰。
- 纯重构,不改变任何用户可见行为:搜索/筛选/分页/"当前会话"高亮/关联任务后缀/详情审阅/加载更多回合,一律原样保留。
- 面板"一分为二":`ReviewView`/`ReviewSource`/`review_content`/`review_content_pane`/`review_webview_spec`/`spawn_review_load_conversation` 不移动,继续留在 `workspace.rs`——它们服务核心 Agent 面板的 `ReviewSource::Session` 和本面板的 `ReviewSource::Conversation` 两种来源,是内核基础设施。
- `SessionOpen`/`DetailLoadMore`/`Hover`/`TextInputMenuOpen` 四个变体包进 `conversations::Message`,但由内核 `App::update` 直接拦截处理,不转发进 `conversations::update`(该函数对应 `match` 分支写 `unreachable!(..)`)——同 Usage/Files/Database/Browser 四个试点已验证过的既有拦截写法。
- 异步结果路由不变式:`SessionsRefreshed(ProjectId, ..)` 按消息自带的 `project_id` 走 `with_project`;其余触发(切面板/搜索/筛选/分页)走 `with_focused_project`。
- 不建 `Extension` trait/注册表,不拆独立 crate。
- 每个任务结束都要 `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过。

---

### Task 1: 纯函数迁移——分页/过滤/焦点捕获三件套搬进 `extensions::conversations`

**Files:**
- Create: `crates/dozer-app/src/extensions/conversations.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(加 `pub mod conversations;`)
- Modify: `crates/dozer-app/src/workspace.rs`(加 `use crate::extensions::conversations;`,原地函数体改成调用新模块)
- Modify: `crates/dozer-app/src/main.rs`(两处引用改路径)

**Interfaces:**
- Produces:`pub(crate) const CONVERSATION_PAGE_SIZE: usize`、`pub(crate) fn conversation_visible_count(pages: usize) -> usize`、`pub(crate) fn filter_sessions<'a>(rows: &'a [SessionRow], query: &str, agent: Option<AgentKind>) -> Vec<&'a SessionRow>`、`pub(crate) fn conversation_agents_present(rows: &[SessionRow]) -> Vec<AgentKind>`、`pub fn conversation_search_field_id() -> Id`、`pub fn take_conversation_search_focused() -> bool`、`pub struct CaptureConversationSearchFocus`。

- [ ] **Step 1: 创建文件,写入 `use` 块 + 分页常量/纯函数**

`crates/dozer-app/src/extensions/conversations.rs`:

```rust
// crates/dozer-app/src/extensions/conversations.rs
//! 对话(Conversations)面板列表侧:当前项目全部 session 的搜索/agent 筛选/
//! 客户端分页/卡片列表。命名用复数——`crate::conversation` 已经是历史对话
//! 展示用的中间表示数据层(`SessionRow`/`ConversationMeta` 等,Usage 面板
//! 也用),这里是另一回事,用复数避免和它读混。
//!
//! 详情审阅侧(`ReviewView`/`dozer://review-trace` webview host)不在这个
//! 模块里——它是内核共享基础设施,同时服务核心 Agent 面板的
//! `ReviewSource::Session` 和本面板的 `ReviewSource::Conversation`,继续留在
//! `workspace.rs`(见 docs/superpowers/specs/2026-09-15-conversations-extension-pilot-design.md)。

use crate::conversation::SessionRow;
use dozer_core::protocol::AgentKind;
use iced_widget::core::Rectangle;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use std::sync::Mutex;

/// 会话列表(对话面板扁平列表)一页显示的条数,与 Git Log commit 列表的
/// `COMMIT_PAGE_SIZE` 保持一致。首帧 1 页,点"更多..."页数递增。
pub(crate) const CONVERSATION_PAGE_SIZE: usize = 20;

/// 按 `pages`(点过几次"更多",0 起)算出当前应该显示到第几条。抽成纯函数
/// 方便 headless 单测;视图只把返回值画出来。
pub(crate) fn conversation_visible_count(pages: usize) -> usize {
    (pages + 1) * CONVERSATION_PAGE_SIZE
}

/// 会话列表关键字 + agent 过滤:标题(`display_title`)大小写不敏感子串匹配
/// (空关键字不过滤标题这一维)叠加 agent 精确匹配(`None` = 不限)。
///
/// `pub(crate)` 而不是私有:Task 3 之前,`workspace.rs` 里还没搬走的
/// `conversation_list_pane` 需要跨模块调用它;Task 3 把调用方也搬进本模块
/// 后,两者同处一个文件,可见性不需要再改回私有——保持 `pub(crate)` 不动即可。
pub(crate) fn filter_sessions<'a>(
    rows: &'a [SessionRow],
    query: &str,
    agent: Option<AgentKind>,
) -> Vec<&'a SessionRow> {
    let needle = query.to_lowercase();
    rows.iter()
        .filter(|r| agent.map(|a| r.agent == a).unwrap_or(true))
        .filter(|r| {
            query.is_empty()
                || r.display_title.to_lowercase().contains(&needle)
                || r.summary
                    .as_deref()
                    .is_some_and(|s| s.to_lowercase().contains(&needle))
        })
        .collect()
}

/// 会话列表里出现过的 agent 种类,去重,固定展示顺序。供底部 footbar 画筛选
/// chip;返回空 = 没有会话数据,footbar 不渲染。`pub(crate)` 理由同
/// `filter_sessions`。
pub(crate) fn conversation_agents_present(rows: &[SessionRow]) -> Vec<AgentKind> {
    const ORDER: [AgentKind; 7] = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Codex,
        AgentKind::Kilo,
        AgentKind::V8agent,
        AgentKind::Unknown,
    ];
    ORDER
        .into_iter()
        .filter(|k| rows.iter().any(|r| r.agent == *k))
        .collect()
}

/// 会话列表搜索框(iced 原生 `text_input`)的 `widget::Id`:main.rs 每帧
/// `interface.operate` 用 `CaptureConversationSearchFocus` 问真实焦点态。
pub fn conversation_search_field_id() -> Id {
    Id::new("conversation-search-box")
}

static CONVERSATION_SEARCH_FOCUSED: std::sync::LazyLock<Mutex<bool>> =
    std::sync::LazyLock::new(|| Mutex::new(false));

/// 读走并复位(消费式),避免搜索框不可见的帧卡死上一次 `true` 永久堵死终端
/// 键盘转发。
pub fn take_conversation_search_focused() -> bool {
    std::mem::replace(&mut *CONVERSATION_SEARCH_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureConversationSearchFocus;
impl Operation<()> for CaptureConversationSearchFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&conversation_search_field_id()) {
            *CONVERSATION_SEARCH_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}
```

- [ ] **Step 2: 从 `workspace.rs` 删除已搬走的部分,保留 `CONVERSATION_DETAIL_PAGE_SIZE`**

`workspace.rs` 现有(约 2506-2521 行)：

```rust
pub(crate) const CONVERSATION_PAGE_SIZE: usize = 20;

/// 对话面板 session 详情的首屏回合页大小(2026-08-27):列表分页
/// (`CONVERSATION_PAGE_SIZE`,20)与详情页分页(这里,200)含义不同,不要混用。
pub(crate) const CONVERSATION_DETAIL_PAGE_SIZE: u32 = 200;

pub(crate) fn conversation_visible_count(pages: usize) -> usize {
    (pages + 1) * CONVERSATION_PAGE_SIZE
}
```

删除 `CONVERSATION_PAGE_SIZE` 常量声明和 `conversation_visible_count` 函数(已原样搬进新文件),**保留** `CONVERSATION_DETAIL_PAGE_SIZE`(详情页分页,是内核 `spawn_review_load_conversation`/`ConversationDetailLoadMore` 用的常量,不属于列表侧)不动。

现有(约 2554-2594 行)`filter_sessions`/`conversation_agents_present` 两个函数整段删除(已搬进新文件)。

现有(约 2523-2552 行)`conversation_search_field_id`/`CONVERSATION_SEARCH_FOCUSED`/`take_conversation_search_focused`/`CaptureConversationSearchFocus` 整段删除(已搬进新文件)。

- [ ] **Step 3: `workspace.rs` 顶部加 `use`,修正剩余调用点的路径**

在 `use crate::extensions::browser;` 之后插入(按字母序):

```rust
use crate::extensions::browser;
use crate::extensions::conversations;
use crate::extensions::database;
```

`workspace.rs` 里仍保留在原处的 `conversation_list_pane`(此时还没搬,Task 3 才搬)函数体内,把:

```rust
let agents_present = conversation_agents_present(rows);
let filtered = filter_sessions(rows, &ws.conversation_search, ws.conversation_agent_filter);
```

改成:

```rust
let agents_present = conversations::conversation_agents_present(rows);
let filtered = conversations::filter_sessions(rows, &ws.conversation_search, ws.conversation_agent_filter);
```

（Step 1 已经把这两个函数定义成 `pub(crate) fn`,所以这里能跨模块调用,不会报"private
function"编译错误。）

`conversation_list_pane` 函数体内其余两处调用点同样加前缀:

```rust
Some(conversation_search_field_id()),
```

→ 两处出现处(搜索框 `Id` 参数、`TextInputMenuOpen` 目标 `id` 字段)都改成:

```rust
Some(conversations::conversation_search_field_id()),
```

```rust
let visible = conversation_visible_count(ws.conversation_pages);
```

改成:

```rust
let visible = conversations::conversation_visible_count(ws.conversation_pages);
```

- [ ] **Step 4: `main.rs` 两处路径更新**

```rust
&mut workspace::CaptureConversationSearchFocus,
```

改成:

```rust
&mut extensions::conversations::CaptureConversationSearchFocus,
```

```rust
workspace::take_conversation_search_focused()
```

改成:

```rust
extensions::conversations::take_conversation_search_focused()
```

- [ ] **Step 5: 声明模块**

`crates/dozer-app/src/extensions.rs`,按字母序插入(在 `browser` 之后、`database` 之前):

```rust
pub mod browser;
pub mod conversations;
pub mod database;
```

- [ ] **Step 6: 编译确认**

```bash
cargo build -p dozer-app
```

Expected: 编译通过,行为不变(只是函数搬了文件位置)。

- [ ] **Step 7: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/extensions/conversations.rs crates/dozer-app/src/extensions.rs \
  crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "refactor(dozer-app): move conversation list pagination/filter/focus helpers into extensions::conversations"
```

---

### Task 2: 自己的 `Message`/`WorkspaceState`/`update`/`spawn_refresh`

**Files:**
- Modify: `crates/dozer-app/src/extensions/conversations.rs`

**Interfaces:**
- Consumes:Task 1 的 `SessionRow`(来自 `crate::conversation`)。
- Produces:`pub enum Message`(11 个变体,清单见下)、`pub struct WorkspaceState`(`sessions()`/`search_focused()`/`set_search_focused()` 三个 `pub` 访问器,其余字段私有)、`pub fn update(ws_state: &mut WorkspaceState, msg: Message)`、`pub fn spawn_refresh(project_id: i64, cwd: PathBuf, client: &dozer_client::Client, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`。

- [ ] **Step 1: `WorkspaceState`**

在文件顶部 `use` 块之后插入:

```rust
use crate::app::{HoverId, ProjectId, TextInputTarget};
use std::path::PathBuf;

/// 挂在每个 Workspace 上的对话面板列表侧状态,对应现有 `Workspace` 上 7 个
/// 平铺的 `conversation_*` 字段。
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
    /// 供内核 `App::conversation_session_open` 按 `conversation_id` 查找
    /// 对应行的总结信息用。
    pub fn sessions(&self) -> Option<&[SessionRow]> {
        self.sessions.as_deref()
    }

    /// 会话列表搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn search_focused(&self) -> bool {
        self.search_focused
    }

    /// 每帧渲染循环读走 `CaptureConversationSearchFocus` 查到的真实焦点态后
    /// 写回这里。
    pub fn set_search_focused(&mut self, focused: bool) {
        self.search_focused = focused;
    }
}
```

- [ ] **Step 2: `Message`**

紧接着插入:

```rust
#[derive(Debug, Clone)]
pub enum Message {
    /// 会话列表刷新结果:当前项目全部 session(联查总结),按最后活跃时间
    /// 倒序。按自带的 `ProjectId` 走 `with_project` 路由。
    SessionsRefreshed(ProjectId, Result<Vec<SessionRow>, String>),
    /// 点击某一行打开该 session 的详情审阅。由内核 `App::update` 直接拦截
    /// 处理(要写进跨面板共享的 `ws.review`),不会到达 `update()`。
    SessionOpen(String, AgentKind),
    /// 详情页"加载更多"追加下一页回合。同上,内核直接拦截处理。
    DetailLoadMore(String, i64),
    /// 列表底部"更多..."翻页,纯客户端状态。
    ListMore,
    /// 搜索框草稿变化。
    SearchInput(String),
    /// 回车/点搜索按钮,草稿落成生效过滤词并重置分页。
    SearchSubmit,
    /// agent 筛选下拉某一项点击,`None` = 全部;重置分页并收起下拉。
    AgentFilterSelect(Option<AgentKind>),
    /// 展开 agent 筛选下拉。
    AgentPickerOpen,
    /// 收起 agent 筛选下拉。
    AgentPickerClose,
    /// 搜索框/翻页按钮的 hover 进度条动画。由内核直接拦截转发给
    /// `App::set_hover`(同 `usage::Message::Hover` 的既有写法),不会到达
    /// `update()`。
    Hover(HoverId, bool),
    /// 搜索框右键菜单目标。由内核直接拦截转发给顶层
    /// `Message::TextInputMenuOpen`(同 `files::Message::TextInputMenuOpen`
    /// 的既有写法),不会到达 `update()`。
    TextInputMenuOpen(TextInputTarget),
}
```

- [ ] **Step 3: `update`**

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
        Message::SessionOpen(..)
        | Message::DetailLoadMore(..)
        | Message::Hover(..)
        | Message::TextInputMenuOpen(..) => {
            unreachable!("由内核 App::update 直接拦截处理,不会转发到这里")
        }
    }
}
```

- [ ] **Step 4: `spawn_refresh`**

```rust
/// 加载(或刷新)当前项目的会话列表(联查总结,标题+摘要预览)。签名/实现
/// 照搬现有 `Workspace::spawn_conversations_refresh`,`client` 参数传入方式
/// 对齐 Usage 试点既有 `usage::spawn_refresh`。
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
            .map(|rows| {
                rows.iter()
                    .map(|(c, s)| SessionRow::from_row(c, s.as_ref()))
                    .collect()
            })
            .map_err(|e| e.to_string());
        emit(Message::SessionsRefreshed(project_id, result));
    });
}
```

- [ ] **Step 5: 编译确认**

```bash
cargo build -p dozer-app
```

Expected: 编译通过。`Message`/`WorkspaceState`/`update`/`spawn_refresh` 目前仍未被内核调用,`dead_code` 警告可接受,不允许报错。

- [ ] **Step 6: 新增单测**

在 `extensions/conversations.rs` 底部追加:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_submit_commits_draft_and_resets_pages() {
        let mut ws_state = WorkspaceState {
            search_draft: "foo".into(),
            pages: 3,
            ..WorkspaceState::default()
        };
        update(&mut ws_state, Message::SearchSubmit);
        assert_eq!(ws_state.search, "foo");
        assert_eq!(ws_state.pages, 0);
    }

    #[test]
    fn agent_filter_select_resets_pages_and_closes_picker() {
        let mut ws_state = WorkspaceState {
            pages: 2,
            agent_picker_open: true,
            ..WorkspaceState::default()
        };
        update(
            &mut ws_state,
            Message::AgentFilterSelect(Some(AgentKind::Claude)),
        );
        assert_eq!(ws_state.agent_filter, Some(AgentKind::Claude));
        assert_eq!(ws_state.pages, 0);
        assert!(!ws_state.agent_picker_open);
    }

    #[test]
    fn sessions_refreshed_err_lands_as_empty_vec_not_none() {
        let mut ws_state = WorkspaceState::default();
        update(
            &mut ws_state,
            Message::SessionsRefreshed(1, Err("boom".into())),
        );
        assert_eq!(ws_state.sessions(), Some([].as_slice()));
    }

    #[test]
    fn filter_sessions_matches_title_case_insensitive() {
        let row = SessionRow {
            conversation_id: "c1".into(),
            agent: AgentKind::Claude,
            last_ts: 0,
            display_title: "Fix Login Bug".into(),
            summary: None,
            summary_status: None,
            task_id: None,
        };
        let rows = vec![row];
        assert_eq!(filter_sessions(&rows, "login", None).len(), 1);
        assert_eq!(filter_sessions(&rows, "nomatch", None).len(), 0);
    }
}
```

（`SessionRow` 字段清单核对自 `crates/dozer-app/src/conversation.rs:31-42` 的实际定义:
`conversation_id`/`agent`/`last_ts`/`display_title`/`summary`/`summary_status`/`task_id` 共 7 个,
`summary_status: Option<SummaryStatus>` 一项本次测试用不到,填 `None`。）

- [ ] **Step 7: 跑测试**

```bash
cargo test -p dozer-app conversations::
```

Expected: 4 个新测试全部 PASS。

- [ ] **Step 8: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/extensions/conversations.rs
git commit -m "feat(dozer-app): add conversations Message/WorkspaceState/update/spawn_refresh"
```

---

### Task 3: 迁移 `view`(列表侧)

**Files:**
- Modify: `crates/dozer-app/src/extensions/conversations.rs`

**Interfaces:**
- Consumes:Task 2 的 `Message`/`WorkspaceState`。
- Produces:`pub fn view<'a>(app: &'a App, ws_state: &'a WorkspaceState, todo_items: &'a [dozer_core::protocol::TodoInfo], open_transcript_paths: &'a [String], width: Length, outer: Border) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>`。

这一步只**新增**函数,`workspace.rs` 里同名的旧函数(`conversation_list_pane`/
`conversation_footer_bar`/`conversation_agent_picker_view`)暂不删除、暂不改调用点——
仍然是当前唯一被 `app.rs` 渲染路径实际调用的版本。新函数此刻是 dead code(允许),
Task 4 才会把旧函数删除、调用点切换过来。这样每一步都能独立编译通过。

- [ ] **Step 1: 补充 `use`**

在文件顶部 `use` 块追加:

```rust
use crate::app::App;
use crate::homespace::home_panel_head;
use crate::theme;
use crate::workspace::{agent_dot_color, agent_icon, lh, relative_time_text};
use byteui::interaction::icons;
use byteui::interaction::icons::IconKind;
use dozer_core::protocol::TodoInfo;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, text};
```

- [ ] **Step 2: `footer_bar`(原 `conversation_footer_bar`)**

```rust
/// 会话列表底部 agent 筛选栏:样式对齐文件树面板的分支切换下拉——左边图标
/// + 当前筛选(agent 名称,不筛选时"全部"),右边一个展开/收起下拉的箭头
/// 按钮。`agents` 为空(没有会话数据)时不渲染整条 bar。
fn footer_bar<'a>(
    ws_state: &WorkspaceState,
    agents: &[AgentKind],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if agents.is_empty() {
        return column![].into();
    }

    let current_label = ws_state
        .agent_filter
        .map(|a| a.label().to_string())
        .unwrap_or_else(|| "全部".to_string());
    let switch = button(icons::view(
        if ws_state.agent_picker_open {
            IconKind::ChevronUp
        } else {
            IconKind::ChevronDown
        },
        byteui::theme::icon_size::row(),
        byteui::theme::color::current().cream,
    ))
    .on_press(if ws_state.agent_picker_open {
        Message::AgentPickerClose
    } else {
        Message::AgentPickerOpen
    })
    .padding(6)
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: byteui::theme::color::current().cream,
        border: iced_widget::core::Border {
            color: byteui::theme::color::current().border,
            width: 0.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    });

    let bar = row![
        icons::view(
            IconKind::Bot,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream,
        ),
        text(current_label)
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().cream),
        iced_widget::space::horizontal(),
        switch,
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let top_line = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..container::Style::default()
        });

    container(column![top_line, bar].spacing(4))
        .width(Length::Fill)
        .padding([6, 0])
        .style(|_t: &iced_widget::Theme| container::Style {
            background: None,
            ..container::Style::default()
        })
        .into()
}
```

- [ ] **Step 3: `agent_picker_view`(原 `conversation_agent_picker_view`)**

```rust
/// `footer_bar` 展开的下拉弹出层:列出"全部" + `agents`,点某项即
/// `AgentFilterSelect` 切换并收起(本地 `stack!` 叠在面板自己内容之上,不是
/// 全窗 overlay)。未展开时返回零高度元素。
fn agent_picker_view(
    ws_state: &WorkspaceState,
    agents: &[AgentKind],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !ws_state.agent_picker_open {
        return iced_widget::Space::new().into();
    }
    let is_all_current = ws_state.agent_filter.is_none();
    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        Vec::new();
    items.push(crate::menu::item_row_fill(
        None,
        "全部",
        if is_all_current {
            byteui::theme::color::current().gold
        } else {
            byteui::theme::color::current().body
        },
        (!is_all_current).then_some(Message::AgentFilterSelect(None)),
    ));
    for &agent in agents {
        let is_current = ws_state.agent_filter == Some(agent);
        let color = if is_current {
            byteui::theme::color::current().gold
        } else {
            byteui::theme::color::current().body
        };
        let leading = icons::view(
            agent_icon(agent),
            byteui::theme::icon_size::row(),
            agent_dot_color(agent),
        );
        items.push(crate::menu::item_row_fill(
            Some(leading),
            agent.label(),
            color,
            (!is_current).then_some(Message::AgentFilterSelect(Some(agent))),
        ));
    }
    let panel: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        crate::menu::shell_frosted(items, Length::Fill);

    let dismiss = MouseArea::new(
        iced_widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::AgentPickerClose);
    let positioned = column![iced_widget::Space::new().height(Length::Fill), panel]
        .width(Length::Fill)
        .height(Length::Fill);
    iced_widget::stack![dismiss, positioned]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
```

- [ ] **Step 4: `view`(原 `conversation_list_pane`)**

```rust
/// 对话列表面板(右面板区"对话"视图的列表侧):当前项目全部 session 的
/// 列表(标题+总结预览),按最后活跃时间倒序。点一行 → `SessionOpen` 驱动
/// 内核加载右侧审阅内容。列表按 `conversation_visible_count` 客户端分页。
///
/// `todo_items`/`open_transcript_paths` 由内核跨读传入(见设计文档"关键
/// 语义确认"——这不是 extension 间耦合,`conversations` 模块本身不持有
/// `todo::WorkspaceState`,只是内核在组装这次 `view` 调用时顺带传了两份
/// 只读数据)。
pub fn view<'a>(
    app: &'a App,
    ws_state: &'a WorkspaceState,
    todo_items: &'a [TodoInfo],
    open_transcript_paths: &'a [String],
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::conversation_list_pane();
    let mut content =
        column![home_panel_head(IconKind::BotMessageSquare, "会话"),].spacing(region.gap);

    let search_active = ws_state.search_focused || !ws_state.search.is_empty();
    let search_box = byteui::form::search_box::view(
        "搜索会话标题/摘要…",
        &ws_state.search_draft,
        Some(conversation_search_field_id()),
        search_active,
        Message::SearchInput,
        Message::SearchSubmit,
        app.hover_progress(HoverId::ConversationSearchSubmit),
        |hovered| Message::Hover(HoverId::ConversationSearchSubmit, hovered),
    );
    content = content.push(byteui::interaction::context_menu::wrap(
        search_box,
        Some(Message::TextInputMenuOpen(TextInputTarget {
            id: conversation_search_field_id(),
            secure: false,
        })),
    ));

    let Some(rows) = ws_state.sessions.as_ref() else {
        content = content.push(lh(text("加载中…")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim)));
        return container(content.padding(region.padding))
            .width(width)
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: region.background.map(Into::into),
                border: outer,
                ..container::Style::default()
            })
            .into();
    };

    let agents_present = conversation_agents_present(rows);
    let filtered = filter_sessions(rows, &ws_state.search, ws_state.agent_filter);
    if rows.is_empty() {
        content = content.push(lh(text("暂无对话记录")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim)));
    } else if filtered.is_empty() {
        content = content.push(lh(text("无匹配结果")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim)));
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mut cards = column![].spacing(region.gap);
    let visible = conversation_visible_count(ws_state.pages);
    for g in filtered.iter().take(visible) {
        let current =
            crate::conversation::is_current_conversation_id(&g.conversation_id, open_transcript_paths);
        let agent_label = g.agent.label();
        let task_suffix = g
            .task_id
            .and_then(|task_id| {
                todo_items
                    .iter()
                    .find(|t| t.id == task_id)
                    .map(|t| t.text.as_str())
            })
            .map(|text| {
                let truncated: String = text.chars().take(30).collect();
                if text.chars().count() > 30 {
                    format!(" · 关联任务:{truncated}…")
                } else {
                    format!(" · 关联任务:{truncated}")
                }
            })
            .unwrap_or_default();
        let sub = if current {
            format!(
                "● 当前 · {agent_label} · {}{task_suffix}",
                relative_time_text(g.last_ts, now_ms)
            )
        } else {
            format!(
                "{agent_label} · {}{task_suffix}",
                relative_time_text(g.last_ts, now_ms)
            )
        };
        let sub_color = if current {
            byteui::theme::color::current().green
        } else {
            byteui::theme::color::current().dim
        };
        let row_btn = button(
            row![
                icons::view(
                    agent_icon(g.agent),
                    byteui::theme::icon_size::row(),
                    agent_dot_color(g.agent),
                ),
                column![
                    lh(text(g.display_title.clone())
                        .size(byteui::theme::font::body())
                        .color(byteui::theme::color::current().cream)),
                    lh(text(sub)
                        .size(byteui::theme::font::caption_sm())
                        .color(sub_color)),
                ]
                .spacing(4),
            ]
            .spacing(8)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .on_press(Message::SessionOpen(g.conversation_id.clone(), g.agent))
        .width(Length::Fill)
        .padding(10)
        .style(byteui::interaction::cards::button_card(
            current,
            byteui::theme::color::current().card,
        ));
        cards = cards.push(row_btn);
    }
    if filtered.len() > visible {
        let more_button = icons::icon_button_entry(
            icons::IconKind::Ellipsis,
            byteui::theme::icon_size::row(),
            false,
            false,
            app.hover_progress(HoverId::ConversationListMore),
            false,
            byteui::theme::geometry::tab_button_size(),
            true,
            Message::ListMore,
            |hovered| Message::Hover(HoverId::ConversationListMore, hovered),
            "更多",
        );
        cards = cards.push(
            container(more_button)
                .width(Length::Fill)
                .align_x(iced_widget::core::alignment::Horizontal::Center),
        );
    }
    content = content.push(
        Scrollable::new(cards)
            .width(Length::Fill)
            .height(Length::Fill)
            .direction(scrollable::Direction::Vertical(
                byteui::interaction::scrollbar::scrollbar(),
            ))
            .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
    );
    content = content.push(footer_bar(ws_state, &agents_present));

    let panel = container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        });

    iced_widget::stack![panel, agent_picker_view(ws_state, &agents_present)]
        .width(width)
        .height(Length::Fill)
        .into()
}
```

（`crate::conversation::is_current_conversation_id` 直接全路径调用,不加 `use`——避免和本模块自己的
`use crate::conversation::SessionRow;` 混在一起时读者要分辨"这个 `conversation` 是谁"；这也是选复数
`conversations` 当模块名的原因之一,全路径写法在这里反而更清楚。）

- [ ] **Step 5: 编译确认**

```bash
cargo build -p dozer-app
```

Expected: 编译通过。`view`/`footer_bar`/`agent_picker_view` 目前仍未被调用,`dead_code` 警告可接受
(注意:`footer_bar`/`agent_picker_view` 是私有 `fn`,只有 `view` 调用它们,`view` 本身是 `pub fn`
但同样没人调用——三者的 dead_code 警告可能因为"有内部调用者"而不完全一致,这是正常现象,不代表
出错)。

- [ ] **Step 6: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/extensions/conversations.rs
git commit -m "feat(dozer-app): add conversations::view (list side), old workspace.rs version still wired"
```

---

### Task 4: 内核接线——切换调用点、删除旧代码

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes:Task 1-3 的 `conversations::{Message, WorkspaceState, update, spawn_refresh, view}`。

- [ ] **Step 1: `Workspace` 结构体字段合并**

`workspace.rs` 里 `pub struct Workspace { .. }` 现有 7 个字段(约 391-417 行,含各自文档注释)整段删除:

```rust
pub(crate) conversation_sessions: Option<Vec<SessionRow>>,
pub(crate) conversation_pages: usize,
pub(crate) conversation_search: String,
pub(crate) conversation_search_draft: String,
pub(crate) conversation_search_focused: bool,
pub(crate) conversation_agent_filter: Option<AgentKind>,
pub(crate) conversation_agent_picker_open: bool,
```

加:

```rust
/// 对话面板列表侧状态——见 `extensions::conversations::WorkspaceState`。
pub(crate) conversations: conversations::WorkspaceState,
```

`Workspace::empty_for_project_placeholder()` 里现有 7 行:

```rust
conversation_sessions: None,
conversation_pages: 0,
conversation_search: String::new(),
conversation_search_draft: String::new(),
conversation_search_focused: false,
conversation_agent_filter: None,
conversation_agent_picker_open: false,
```

改成一行(紧邻既有 `usage: usage::WorkspaceState::default(),` 之前或之后均可,保持字段声明顺序对应
即可):

```rust
conversations: conversations::WorkspaceState::default(),
```

`Workspace::adopt_project()` 里现有 7 行:

```rust
self.conversation_sessions = None;
self.conversation_pages = 0;
self.conversation_search.clear();
self.conversation_search_draft.clear();
self.conversation_search_focused = false;
self.conversation_agent_filter = None;
self.conversation_agent_picker_open = false;
```

改成一行:

```rust
self.conversations = conversations::WorkspaceState::default();
```

- [ ] **Step 2: 删除 `workspace.rs` 里已被取代的旧函数**

整段删除(Task 3 已在 `extensions::conversations` 里写好等价版本):
- `conversation_list_pane`(约 2740-2936 行)
- `conversation_footer_bar`(约 2600-2669 行)
- `conversation_agent_picker_view`(约 2676-2731 行)

（三者相邻,`conversation_agent_picker_view` 结尾紧接着就是 `conversation_list_pane` 开头,删除时
用实际文件里这三个函数各自的闭合大括号定位边界,不要只信行号——行号会因为 Task 1-3 的改动而漂移。
`group_tabs_by_agent` 函数(紧跟在 `conversation_list_pane` 之后)是 Agent 核心面板用的,不属于
这次删除范围,务必确认删除边界停在 `conversation_list_pane` 自己的 `}` 上。）

- [ ] **Step 3: `spawn_conversations_refresh` 方法改成薄封装**

`workspace.rs` 现有 `pub(crate) fn spawn_conversations_refresh(&self, io: &ShellIo)` 方法体:

```rust
pub(crate) fn spawn_conversations_refresh(&self, io: &ShellIo) {
    let Some(project) = self.project.as_ref() else {
        return;
    };
    let project_id = project.id;
    let cwd = PathBuf::from(&project.path);
    let client = io.client.clone();
    let proxy = io.proxy.clone();
    io.handle.spawn(async move {
        let result = client
            .list_conversations_with_summaries(&cwd.to_string_lossy(), None, 500, 0)
            .await
            .map(|rows| {
                rows.iter()
                    .map(|(c, s)| SessionRow::from_row(c, s.as_ref()))
                    .collect()
            })
            .map_err(|e| e.to_string());
        let _ = proxy.send_event(Message::ConversationSessionsRefreshed(project_id, result));
    });
}
```

改成薄封装,委托给 `conversations::spawn_refresh`(既有调用点 `ws.spawn_conversations_refresh(io)`
不用跟着改):

```rust
pub(crate) fn spawn_conversations_refresh(&self, io: &ShellIo) {
    let Some(project) = self.project.as_ref() else {
        return;
    };
    let project_id = project.id;
    let cwd = PathBuf::from(&project.path);
    let proxy = io.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::Conversations(m));
    };
    conversations::spawn_refresh(project_id, cwd, &io.client, &io.handle, emit);
}
```

- [ ] **Step 4: `review_content`/`load_more_button` 里的消息引用**

`workspace.rs::load_more_button`:

```rust
.on_press(Message::ConversationDetailLoadMore(
    conversation_id.to_string(),
    after_turn_index,
))
```

改成:

```rust
.on_press(Message::Conversations(conversations::Message::DetailLoadMore(
    conversation_id.to_string(),
    after_turn_index,
)))
```

（`review_content`/`review_content_pane`/`ReviewView`/`ReviewSource` 等其余部分不动。）

- [ ] **Step 5: `app.rs` 顶层 `Message` 枚举**

在 `use crate::extensions::browser;` 之后插入(按字母序,与 `workspace.rs` Task 1 Step 3 同样位置
逻辑):

```rust
use crate::extensions::browser;
use crate::extensions::conversations;
use crate::extensions::database;
```

删除 9 个平铺变体(各自散落在枚举定义中,连同其文档注释一起删):
`ConversationSessionsRefreshed(ProjectId, Result<Vec<SessionRow>, String>)`、
`ConversationSessionOpen(String, AgentKind)`、`ConversationDetailLoadMore(String, i64)`、
`ConversationListMore`、`ConversationSearchInput(String)`、`ConversationSearchSubmit`、
`ConversationAgentFilterSelect(Option<AgentKind>)`、`ConversationAgentPickerOpen`、
`ConversationAgentPickerClose`。

在 `Usage(usage::Message),` 变体之后插入:

```rust
Usage(usage::Message),
/// 对话面板的全部消息,内核只转发/特案拦截,不解读业务逻辑——见
/// `extensions::conversations::Message`。
Conversations(conversations::Message),
```

- [ ] **Step 6: `app.rs::update()` 里的 Conversation 相关分支**

删除现有 9 个平铺分支(`Message::ConversationSessionsRefreshed`/`ConversationSessionOpen`/
`ConversationDetailLoadMore`/`ConversationListMore`/`ConversationSearchInput`/
`ConversationSearchSubmit`/`ConversationAgentFilterSelect`/`ConversationAgentPickerOpen`/
`ConversationAgentPickerClose`,约 4706-4780 行)。

加 6 支(**顺序有意义**——前 4 支内核专属拦截必须排在通配的
`Message::Conversations(msg @ ..)`/`Message::Conversations(msg)` 之前,否则会被后者提前吃掉,
Rust `match` 按顺序取第一个匹配分支):

```rust
Message::Conversations(conversations::Message::SessionOpen(conversation_id, agent)) => {
    self.conversation_session_open(conversation_id, agent);
}
Message::Conversations(conversations::Message::DetailLoadMore(
    conversation_id,
    after_turn_index,
)) => {
    self.with_focused_project(|ws, io| {
        ws.spawn_review_load_conversation(
            io,
            conversation_id,
            after_turn_index,
            CONVERSATION_DETAIL_PAGE_SIZE,
            true,
        );
    });
}
Message::Conversations(conversations::Message::Hover(id, h)) => self.set_hover(id, h),
Message::Conversations(conversations::Message::TextInputMenuOpen(target)) => {
    self.update(Message::TextInputMenuOpen(target));
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

- [ ] **Step 7: `conversation_session_open` 方法**

`app.rs` 现有方法体里:

```rust
let Some(row) = ws
    .conversation_sessions
    .as_ref()
    .and_then(|rows| rows.iter().find(|r| r.conversation_id == conversation_id))
else {
```

改成:

```rust
let Some(row) = ws
    .conversations
    .sessions()
    .and_then(|rows| rows.iter().find(|r| r.conversation_id == conversation_id))
else {
```

- [ ] **Step 8: `PanelKind::Conversations` 的 `view()` 渲染调用点**

`app.rs` 现有:

```rust
let list = conversation_list_pane(
    app,
    ws,
    Length::FillPortion(list_portion),
    zone_pane_border(zone, rc),
);
```

改成(`ws.open_transcript_paths()` 返回一个新分配的 `Vec<String>`,不能直接 `&ws.open_transcript_paths()`
取引用传参——临时值活不过这个表达式,必须先落一个局部变量):

```rust
let open_paths = ws.open_transcript_paths();
let list = conversations::view(
    app,
    &ws.conversations,
    ws.todo.items(),
    &open_paths,
    Length::FillPortion(list_portion),
    zone_pane_border(zone, rc),
)
.map(Message::Conversations);
```

- [ ] **Step 9: `App::conversation_search_focused`/`set_conversation_search_focused`**

```rust
pub fn conversation_search_focused(&self) -> bool {
    self.active_workspace()
        .is_some_and(|ws| ws.conversation_search_focused)
}

pub fn set_conversation_search_focused(&mut self, focused: bool) {
    if let Some(ws) = self.active_workspace_mut() {
        ws.conversation_search_focused = focused;
    }
}
```

改成:

```rust
pub fn conversation_search_focused(&self) -> bool {
    self.active_workspace()
        .is_some_and(|ws| ws.conversations.search_focused())
}

pub fn set_conversation_search_focused(&mut self, focused: bool) {
    if let Some(ws) = self.active_workspace_mut() {
        ws.conversations.set_search_focused(focused);
    }
}
```

- [ ] **Step 10: 编译,逐条修正遗漏引用**

```bash
cargo build -p dozer-app
```

Expected: 可能出现遗漏的 `ws.conversation_sessions`/`ws.conversation_pages`/
`Message::ConversationXxx` 引用(比如调试日志、`RestorePayload`/快照恢复逻辑里如果有直接构造这些
字段的地方)。逐条改成 `ws.conversations.xxx()`(读写走 Task 2 定义的访问器,没有访问器的私有字段
如果确实需要内核读写,回到 Task 2 补一个;当前设计只预留了 `sessions()`/`search_focused()`/
`set_search_focused()` 三个,若编译报错指向其它字段,先确认是不是真的需要跨模块访问,再决定加访问
器还是这处引用本身就该删除)。

- [ ] **Step 11: 全量测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt
```

Expected: 全绿。

- [ ] **Step 12: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/app.rs
git commit -m "refactor(dozer-app): route Conversations panel through extensions::conversations"
```

---

### Task 5: 全量校验与人工验收

**Files:** 无代码改动。

- [ ] **Step 1: 全 workspace 构建 + 测试 + clippy + fmt**

```bash
cargo build
cargo test -p dozer-app
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全绿。

- [ ] **Step 2: 人工验收(`cargo run -p dozer-app`)**

对照 Conversations 面板现有行为逐项走一遍,确认拆分没有改变任何可见行为:
- 切到"会话"面板(右图标栏),触发一次列表刷新,展示会话卡片(标题+agent名+相对时间)。
- 搜索框输入关键字 + 回车,列表按标题/摘要过滤;清空搜索词恢复全部。
- 点开底部 agent 筛选下拉,选某个 agent,列表按 agent 过滤,分页重置;再切回"全部"。
- 列表条数超过一页时,点"更多..."翻页,不重新问 daemon 要数据。
- 点一条会话卡片,右侧详情审阅区展开该 session 的回合列表 + 总结标题/全文;详情页滚到底点
  "加载更多…"追加下一页回合。
- 有关联 Todo 任务的会话,副行显示"· 关联任务:xxx"后缀。
- 切换项目页签,各项目的会话列表/搜索词/筛选/分页状态互不干扰。
- 搜索框聚焦时,`main.rs` 键盘路由仍能正常打字(不会被终端抢走按键)。
- 搜索框右键仍能弹出复制/粘贴菜单。

- [ ] **Step 3: 确认没有遗留未提交的改动**

```bash
git status
```

Expected: 干净,无未跟踪/未暂存改动(`.dozer/` 目录等项目运行时产物除外)。

- [ ] **Step 4: 提请代码审阅,审阅通过后合并 `feature/conversations-extension-pilot` 到 `main`**

不要自行合并——按 Global Constraints 的要求,新开的独立分支需要经过审阅确认后才能合并主干。
