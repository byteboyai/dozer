# Usage(用量面板)面板扩展化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `crates/dozer-app/src/usage.rs`(用量统计的数据层+视图层,目前直接用顶层 `Message`)拆成自洽模块 `extensions::usage`(自己的 `Message`/`WorkspaceState`/`update`/`spawn_refresh`/`view`),`workspace.rs` 内核只留包装转发——阶段 1 扩展化重构的第五个试点,也是形态最简单的一个:两个纯 per-project 字段,不需要 `AppState`。

**Architecture:** `usage.rs` 整体搬进 `crates/dozer-app/src/extensions/usage.rs`。`WorkspaceState`(挂 `Workspace`,对应现有 `usage`/`usage_loading` 两个字段)、`Message`(`Refresh`/`Loaded` 两个变体)、`update`(处理这两条消息)、`spawn_refresh`(自由函数,替代现有 `Workspace::spawn_usage_refresh` 的内部实现,该方法保留为薄封装——同 Files 试点 `spawn_project_git_refresh` 的处理方式)。`view` 的 `rows`/`loading` 两个独立参数合并成 `ws_state: &WorkspaceState`。

**Tech Stack:** Rust workspace;iced 0.14;`tokio::runtime::Handle` + `emit: impl Fn(Message) + Send + 'static` 回调风格(同前四个试点)。

## Global Constraints

- 纯重构,不改变任何用户可见行为——统计中/空态文案、汇总卡片、柱状图/饼图/分组列表渲染、"只在切到面板或点刷新按钮时才重新扫"的节流规则,一律原样保留。
- 不建 `Extension` trait/注册表,不拆独立 crate。
- `Loaded` 是异步结果,必须自带 `ProjectId` 按 `with_project` 路由,不能走 `with_focused_project`(同 Files/Todo/浏览器已验证的不变式)。
- 不新增 `AppState`——两个字段都是纯 per-project 数据。
- 每个任务结束都要 `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过。
- 设计文档:`docs/superpowers/specs/2026-08-07-usage-extension-pilot-design.md`(有疑问以它为准)。

---

### Task 1: 纯迁移——`extensions::usage` 建文件、搬运全部内容

**Files:**
- Create: `crates/dozer-app/src/extensions/usage.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(加 `pub mod usage;`)
- Modify: `crates/dozer-app/src/main.rs`(删 `mod usage;`)
- Modify: `crates/dozer-app/src/workspace.rs`(`use crate::usage;` 改 `use crate::extensions::usage;`)
- Delete: `crates/dozer-app/src/usage.rs`

**Interfaces:**
- Produces:与现有 `usage.rs` 完全相同的公开符号(`ConversationUsage`/`ProjectUsageTotals`/
  `DayAgentTotals`/`parse_usage`/`aggregate`/`group_usage_by_agent`/
  `daily_totals_by_agent`/`agent_token_share`/`view`),只是挪了文件位置,签名不变
  (`view` 仍吃 `rows`/`loading` 两个独立参数,`use crate::workspace::Message` 也不变
  ——本任务不碰 `Message`,留到 Task 2)。

- [ ] **Step 1: 创建文件,原样整份复制**

把现有 `crates/dozer-app/src/usage.rs`(975 行,完整清单见设计文档"目标"#2:
`ConversationUsage`/`parse_usage`/`parse_claude_shaped_usage`/
`parse_codebuddy_shaped_usage`/`ProjectUsageTotals`/`aggregate`/
`group_usage_by_agent`/`day_index_from_ms`/`civil_from_days`/`DayAgentTotals`/
`daily_totals_by_agent`/`agent_token_share`/`panel_header`/`view`/`summary_card`/
`usage_row`/`grouped_list`/`bar_segment`/`bar_chart`/`format_token_short`/
`PieChart`/`pie_chart`/`chart_legend`,以及它们的 23 个测试)整份复制到
`crates/dozer-app/src/extensions/usage.rs`,一个字不改(含文件顶部的 doc comment,
`use crate::workspace::Message` 这一行本任务不动)。唯一要改的是测试模块里的
fixture 路径:

```rust
let jsonl = include_str!("../../dozer-hook/fixtures/codebuddy-transcript-sample.jsonl");
```

原路径是相对 `crates/dozer-app/src/usage.rs` 算的(`../../dozer-hook/fixtures/...`
= `crates/dozer-hook/fixtures/...`),挪进 `extensions/usage.rs` 后目录深了一层,
改成:

```rust
let jsonl = include_str!("../../../dozer-hook/fixtures/codebuddy-transcript-sample.jsonl");
```

- [ ] **Step 2: 声明模块**

`crates/dozer-app/src/extensions.rs`,按字母序插入(在 `todo` 之后):

```rust
pub mod browser;
pub mod files;
pub mod git_log;
pub mod todo;
pub mod usage;
```

`crates/dozer-app/src/main.rs` 删除 `mod usage;` 一行(现第 22 行)。

`crates/dozer-app/src/workspace.rs` 把 `use crate::usage;`(现第 53 行)改成
`use crate::extensions::usage;`。

- [ ] **Step 3: 删除原文件**

```bash
git rm crates/dozer-app/src/usage.rs
```

- [ ] **Step 4: 编译 + 测试确认纯移动没有破坏任何东西**

```bash
cargo build -p dozer-app
cargo test -p dozer-app usage::
```

Expected: 编译通过,原有 23 个测试原样搬过来全部 PASS(测试名不变,只是模块路径从
`usage::tests::xxx` 变成 `extensions::usage::tests::xxx`)。

- [ ] **Step 5: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/usage.rs crates/dozer-app/src/extensions.rs \
  crates/dozer-app/src/main.rs crates/dozer-app/src/workspace.rs \
  crates/dozer-app/src/usage.rs
git commit -m "refactor(dozer-app): move usage.rs into extensions/usage.rs"
```

---

### Task 2: 自己的 `Message`/`WorkspaceState`/`update`/`spawn_refresh`

**Files:**
- Modify: `crates/dozer-app/src/extensions/usage.rs`

**Interfaces:**
- Consumes:Task 1 迁入的全部纯函数(`parse_usage`/`aggregate`/`daily_totals_by_agent`/
  `agent_token_share`等)。
- Produces:`pub enum Message { Refresh, Loaded(i64, Vec<(ConversationMeta,
  ConversationUsage)>) }`、`pub struct WorkspaceState`(含 `rows()`/`loading()`/
  `set_loading()` 三个方法)、`pub fn update(ws_state: &mut WorkspaceState, msg:
  Message, project_id: i64, project_path: PathBuf, handle: &tokio::runtime::Handle,
  emit: impl Fn(Message) + Send + 'static)`、`pub fn spawn_refresh(project_id: i64,
  project_path: PathBuf, handle: &tokio::runtime::Handle, emit: impl Fn(Message) +
  Send + 'static)`、`pub fn view<'a>(ws_state: &'a WorkspaceState, project_name:
  &'a str, width: Length, outer: Border) -> Element<'a, Message, ..>`。

- [ ] **Step 1: 类型定义,替换 `use crate::workspace::Message`**

`extensions/usage.rs` 顶部把:

```rust
use crate::workspace::Message;
```

改成删除(不再需要这行,本模块马上定义自己的 `Message`),在文件里
`ConversationUsage` 定义之前插入:

```rust
/// 挂在每个 Workspace 上的 Usage 面板状态,对应现有 `Workspace` 上
/// `usage`/`usage_loading` 两个字段。
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
    /// 立即标记"统计中",不等 `spawn_refresh` 的异步结果落地才置真。
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }
}

/// 对应现在顶层 `Message` 里的 `UsageRefresh`/`UsageLoaded` 两个变体,去
/// 前缀原样搬来。
#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    Loaded(i64, Vec<(ConversationMeta, ConversationUsage)>),
}
```

- [ ] **Step 2: `update` + `spawn_refresh`**

在 `agent_token_share` 函数之后、`panel_header` 函数之前插入:

```rust
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

/// 异步扫描项目全部 agent transcript 并逐个解析用量。内核在
/// `RightIconSelect(RightView::Usage)` 分支(切到面板首次刷新)与
/// `update` 处理 `Refresh`(手动点刷新按钮)两处调用。现有
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

`use std::path::PathBuf;` 需要加进文件顶部 `use` 块(现有 `usage.rs` 没有这行,因为
原来 `Workspace::spawn_usage_refresh` 里的 `PathBuf` 用的是 `workspace.rs` 自己的
`use`)。

- [ ] **Step 3: `panel_header`/`view` 改用本模块 `Message`,`view` 合并参数**

`panel_header` 函数体不变(`Message::UsageRefresh` 改成 `Message::Refresh`,
`icons::view::<Message>(..)` 的 `Message` 泛型参数自动跟随文件顶部新定义,不用改
调用写法)。

`view` 签名:

```rust
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    project_name: &'a str,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
```

函数体内 `rows: &[(..)]` 参数改用 `ws_state.rows()`,`loading: bool` 参数改用
`ws_state.loading()`,其余逻辑(`if loading { .. } else if rows.is_empty() { .. }
else { .. }` 三分支)不变。

- [ ] **Step 4: `PieChart` 的 `Message` 类型**

`impl canvas::Program<Message, iced_widget::Theme, iced_widget::Renderer> for
PieChart` 这一行不用改代码——`Message` 标识符已经指向本模块 Step 1 新定义的
`Message`,不再是 `crate::workspace::Message`(Rust 按词法作用域解析,原来的
`use crate::workspace::Message;` 已经在 Step 1 删除,这里的 `Message` 自动绑定到
同文件定义的枚举)。

- [ ] **Step 5: 编译确认**

```bash
cargo build -p dozer-app
```

Expected: 编译通过。`update`/`spawn_refresh`/`WorkspaceState` 目前仍未被内核调用,
`dead_code` 警告可接受,不允许报错。

- [ ] **Step 6: 新增单测**

在 `extensions/usage.rs` 现有 `#[cfg(test)] mod tests` 里追加:

```rust
#[tokio::test]
async fn refresh_sets_loading_true() {
    let mut ws_state = WorkspaceState::default();
    let handle = tokio::runtime::Handle::current();
    update(
        &mut ws_state,
        Message::Refresh,
        1,
        std::path::PathBuf::from("/tmp/does-not-matter"),
        &handle,
        |_| {},
    );
    assert!(ws_state.loading());
}

#[tokio::test]
async fn loaded_clears_loading_and_stores_rows() {
    let mut ws_state = WorkspaceState {
        loading: true,
        ..WorkspaceState::default()
    };
    let handle = tokio::runtime::Handle::current();
    let rows = vec![(
        meta(AgentKind::Claude, "a"),
        ConversationUsage {
            turns: 3,
            ..Default::default()
        },
    )];
    update(
        &mut ws_state,
        Message::Loaded(1, rows.clone()),
        1,
        std::path::PathBuf::from("/tmp/does-not-matter"),
        &handle,
        |_| {},
    );
    assert!(!ws_state.loading());
    assert_eq!(ws_state.rows(), rows.as_slice());
}
```

（`meta`/`AgentKind` 已经是现有测试模块里的既有辅助函数/既有 `use`,不需要新增
`use`;`WorkspaceState` 的 `rows`/`loading` 两个字段在测试里用结构体字面量
`WorkspaceState { loading: true, ..WorkspaceState::default() }` 直接构造是可以的
——测试模块通过 `use super::*;` 能看到私有字段,不需要额外开访问器。）

- [ ] **Step 7: 跑测试**

```bash
cargo test -p dozer-app usage::
```

Expected: 全部测试(23 个既有 + 2 个新增)PASS。

- [ ] **Step 8: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/extensions/usage.rs
git commit -m "feat(dozer-app): add usage Message/WorkspaceState/update/spawn_refresh"
```

---

### Task 3: 内核接线

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes:Task 1-2 的 `usage::WorkspaceState`/`usage::Message`/`usage::update`/
  `usage::spawn_refresh`/`usage::view`。

- [ ] **Step 1: `Workspace` 结构体字段合并**

`pub struct Workspace { .. }` 里删除:

```rust
usage: Vec<(ConversationMeta, usage::ConversationUsage)>,
usage_loading: bool,
```

加:

```rust
/// Usage 面板 per-project 状态——见 `extensions::usage::WorkspaceState`。
usage: usage::WorkspaceState,
```

`Workspace::empty_for_project_placeholder()` 里现有两行:

```rust
usage: Vec::new(),
usage_loading: false,
```

改成一行:

```rust
usage: usage::WorkspaceState::default(),
```

`Workspace::adopt_project()` 里现有两行:

```rust
self.usage = Vec::new();
self.usage_loading = false;
```

改成一行:

```rust
self.usage = usage::WorkspaceState::default();
```

- [ ] **Step 2: `spawn_usage_refresh` 方法改成薄封装**

现有 `fn spawn_usage_refresh(&self, io: &ShellIo)` 方法体:

```rust
fn spawn_usage_refresh(&self, io: &ShellIo) {
    let Some(p) = &self.project else {
        return;
    };
    let project_id = p.id;
    let cwd = PathBuf::from(&p.path);
    let proxy = io.proxy.clone();
    io.handle.spawn(async move {
        let rows = tokio::task::spawn_blocking(move || {
            conversation::list_all_conversations(&cwd)
                .into_iter()
                .filter_map(|meta| {
                    let jsonl = std::fs::read_to_string(&meta.path).ok()?;
                    let u = usage::parse_usage(meta.agent, &jsonl);
                    Some((meta, u))
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        let _ = proxy.send_event(Message::UsageLoaded(project_id, rows));
    });
}
```

改成薄封装,委托给 `usage::spawn_refresh`(同 Files 试点
`Workspace::spawn_project_git_refresh` 改成委托 `files::spawn_git_refresh` 的处理
方式——两个既有调用点 `ws.spawn_usage_refresh(io)`(`UsageRefresh`/
`RightIconSelect(Usage)` 两处)不用跟着改):

```rust
fn spawn_usage_refresh(&self, io: &ShellIo) {
    let Some(p) = &self.project else {
        return;
    };
    let project_id = p.id;
    let project_path = PathBuf::from(&p.path);
    let proxy = io.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::Usage(m));
    };
    usage::spawn_refresh(project_id, project_path, &io.handle, emit);
}
```

- [ ] **Step 3: 顶层 `Message` 枚举**

删除:

```rust
UsageRefresh,
UsageLoaded(ProjectId, Vec<(ConversationMeta, usage::ConversationUsage)>),
```

加:

```rust
/// Usage 面板的全部消息,内核只转发不解读——见 `extensions::usage::Message`。
Usage(usage::Message),
```

- [ ] **Step 4: `update()` 里 Usage 相关分支**

删除现有两支:

```rust
Message::UsageRefresh => {
    self.with_focused_project(|ws, io| {
        ws.usage_loading = true;
        ws.spawn_usage_refresh(io);
    });
}
Message::UsageLoaded(project_id, rows) => {
    self.with_project(project_id, move |ws, _io| {
        ws.usage = rows;
        ws.usage_loading = false;
    });
}
```

加 2 支(`Loaded` 自带 `project_id`,必须走 `with_project`;`Refresh` 走
`with_focused_project`——两支路由方式不同是消息形状决定的,不能合并,见设计文档
"内核侧改动"):

```rust
Message::Usage(msg @ usage::Message::Loaded(project_id, ..)) => {
    self.with_project(project_id, move |ws, io| {
        let Some(project) = &ws.project else { return };
        let project_path = PathBuf::from(&project.path);
        let handle = io.handle.clone();
        let proxy = io.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Usage(m));
        };
        usage::update(&mut ws.usage, msg, project_id, project_path, &handle, emit);
    });
}
Message::Usage(msg) => {
    self.with_focused_project(|ws, io| {
        let Some(project) = &ws.project else { return };
        let project_id = project.id;
        let project_path = PathBuf::from(&project.path);
        let handle = io.handle.clone();
        let proxy = io.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Usage(m));
        };
        usage::update(&mut ws.usage, msg, project_id, project_path, &handle, emit);
    });
}
```

- [ ] **Step 5: `RightIconSelect(RightView::Usage)` 分支**

现有:

```rust
if v == RightView::Usage {
    self.with_focused_project(|ws, io| {
        ws.usage_loading = true;
        ws.spawn_usage_refresh(io);
    });
}
```

`ws.usage_loading = true;` 改成 `ws.usage.set_loading(true);`,`ws.spawn_usage_refresh(io);`
调用不变(Step 2 已经把这个方法内部改成委托 `usage::spawn_refresh`,调用点不用动)。

- [ ] **Step 6: `App::view()` 的 `RightView::Usage` 分支**

现有:

```rust
RightView::Usage => usage::view(
    &ws.usage,
    ws.usage_loading,
    ws.project
        .as_ref()
        .map(|p| p.name.as_str())
        .unwrap_or("未打开项目"),
    Length::Fill,
    zone_pane_border(zone, ac),
),
```

改成:

```rust
RightView::Usage => usage::view(
    &ws.usage,
    ws.project
        .as_ref()
        .map(|p| p.name.as_str())
        .unwrap_or("未打开项目"),
    Length::Fill,
    zone_pane_border(zone, ac),
)
.map(Message::Usage),
```

（去掉单独传的 `ws.usage_loading` 参数——`view` 新签名只吃 `ws_state: &WorkspaceState`
一个参数就能读到 `rows`/`loading` 两者,`unwrap_or("未打开项目")` 兜底原样保留。）

- [ ] **Step 7: 编译,逐条修正**

```bash
cargo build -p dozer-app
```

Expected: 可能出现遗漏的 `ws.usage_loading`/`Message::UsageRefresh`/
`Message::UsageLoaded` 引用(比如某处调试日志或未预料到的第二个 `RightIconSelect`
分支),逐条改成 `ws.usage.xxx()`/`Message::Usage(usage::Message::Xxx)`。

- [ ] **Step 8: 全量测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt
```

Expected: 全绿。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(dozer-app): route Usage messages through extensions::usage"
```

---

### Task 4: 全量校验与人工验收

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

对照 Usage 面板现有行为逐项走一遍,确认拆分没有改变任何可见行为:
- 切到右侧"用量统计"图标(首次切入),面板显示"统计中…",随后展示真实数据或
  "这个项目还没有 agent 对话记录"的空态。
- 点面板头部的刷新按钮,重新触发一次扫描,数据更新。
- 有对话记录时:汇总卡片(轮次/工具调用/触达文件/token 四项)、按天柱状图(Claude/
  CodeBuddy/OpenCode 三色堆叠)、饼图 + 图例、按 agent 分组的会话列表,渲染正常。
- 切换项目页签,用量统计各自独立,不串项目;切到没打开项目的页签,面板显示
  "未打开项目"字样且不 panic。

- [ ] **Step 3: 确认没有遗留未提交的改动**

```bash
git status
```

Expected: 干净,无未跟踪/未暂存改动(`.dozer/` 目录等项目运行时产物除外)。
