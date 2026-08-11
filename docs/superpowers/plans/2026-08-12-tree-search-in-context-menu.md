# 文件树右键"搜索"入口 · 全文搜索弹窗 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在文件树右键菜单**最顶部**加一个"搜索"项；点击后在**当前目录(子树)/文件**范围内
弹出全文搜索窗口（形制参考文件编辑弹层 `edit_modal`），输入关键字做内容子串搜索，列出
`相对路径:行号: 命中行` 列表；点击命中行在内核里打开该文件预览（复用单击文件树行同一条
`OpenFile → PreviewOpenPath` 路径）。这是一次性功能，不加搜索历史/索引/后台预扫描。

**Architecture:** 新文件 `crates/dozer-app/src/extensions/search.rs`，仿 `ssh` 的扩展化
布局，但**不接入 `LeftView`/左侧图标栏**——搜索是瞬态弹窗，不是常驻面板。弹窗状态
（`WorkspaceState`）挂每个 `Workspace`（同 `edit_session` 的归属方式），由 `App::view()`
顶层互斥浮层链的 `stack!` 里以 `search_modal(ws)` 渲染（`edit_modal` 同款 `SCRIM` 遮罩 +
`CARD` 对话框）。触发链：

1. `files::context_menu_popup` 在目录分支（`新建文件`/`新建文件夹`）**之前**插入
   "搜索"项 → `Message::SearchInContextMenu { path, is_dir }`。
2. 该消息由内核拦截（不进 `files::update`，同 `CopyPath` 的既有拦截模式）：
   `context_menu = None`，聚焦 Workspace 上 `search.open = true` + `scope = Dir|File(target)`。
3. 关键字输入走自绘输入框（同 files 顶栏搜索框，键盘由 `main.rs` 拦截层路由，不喂 PTY）；
   提交后 `handle.spawn` 异步跑 `search_scope`，结果 `SearchResults` 回灌。
4. 点击结果 `SearchPick` 由 `main.rs` 拦截映射为既有 `Message::PreviewOpenPath`，关弹窗，
   打开该文件预览。

搜索内核用 `grep` crate（ripgrep 内核，纯 Rust，尊重 `.gitignore`/hidden）——不 spawn
外部 `rg` 二进制、不引入 Node/Python，符合"核心不依赖外部工具/语言"裁决。

**Tech Stack:** Rust workspace；新增 `grep`（含 `grep-searcher`/`grep-regex`/`grep-ignore`）；
`tokio`（已有）跑异步搜索；弹窗 UI 走既有 `iced_widget` stack/`container`/`Scrollable`。

## Global Constraints

- **在独立分支上开发**：建分支 `feature/tree-search-popup`（或对应 worktree），完成后提请
  审阅，通过再合并回 `main`。
- **这个计划基于当前 `main` 分析**。几个会改到同一批壳层符号的并行分支尚未合并是常态——
  开工前先 `grep -n` 核对你实际面对的分支上这些符号的位置与形状，不要假设跟本文档写的一致：
  - `files.rs` 里 `context_menu_popup` 菜单项顺序 / `Message` 变体列表
  - `app.rs` 的 `App::view()` 顶层 `stack![...]` 浮层链里 `edit_modal`/`maximize_overlay`
    实际是哪些元素、`search_modal` 插到哪一层
  - `main.rs` 键盘拦截层里 `files` 搜索框编辑态/编辑弹层的既有按键路由分支（照抄其口径）
  - `extensions.rs` 的 `pub mod` 字母序列表（写计划时是 `acceptance/browser/database/files/
    footbar/git_log/project/ssh/todo/usage`，`search` 排 `project` 和 `ssh` 之间）
- **搜索弹窗不加持久化**（非目标），`.dozer` 下不新增文件；状态只在内存，随各自
  `Workspace` 生命周期。
- **`grep` API 精确名按实装核实**：`grep-searcher`/`grep-regex`/`grep-ignore` 的
  `SearcherBuilder`/`IgnoreBuilder`/`RegexMatcher` 等具体方法名与 `grep` 伞 crate 的
  re-export 路径，以 `cargo add grep` 后实际拉的版本 ± `docs.rs`/本地源码为准——本计划
  代码块给的是结构示意（尤其 Task 2 的 `search_scope`），写代码时对照真实 API 调整，
  **不要因为一个方法名不对就直接绕过"grep 内核做内容匹配"这个设计本身**。
- 每个任务结束都要 `cargo build && cargo test && cargo clippy --all-targets && cargo fmt`
  干净通过（全 workspace）。Task 1-3 新模块可能未接通内核，会有预期 `dead_code` 警告，
  Task 4 解决，不要加 `#[allow(dead_code)]`。
- 设计文档：`docs/superpowers/specs/2026-08-12-tree-search-in-context-menu-design.md`（有
  疑问以它为准）。

---

### Task 1: 新增依赖 + 图标资源

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`
- Create: `crates/dozer-app/assets/icons/search.svg`
- Modify: `crates/dozer-app/src/icons.rs`

**Interfaces:**
- Produces:`icons::IconKind::Search` 变体。

- [ ] **Step 1: 加依赖**

`crates/dozer-app/Cargo.toml` 的 `[dependencies]` 块里加：

```toml
grep = "0.3"
```

（版本号按 `cargo add grep` 实际解出的版本为准，不强求锁死 0.3。`grep` 伞 crate 会一起
拉进 `grep-searcher`/`grep-regex`/`grep-ignore`。不显式开 `-pcre2` feature。先
`grep "grep" crates/dozer-app/Cargo.toml` 核对是否已存在，避免重复加。`ignore` crate
本地 registry 已有（git2/notify 的传递依赖），不一定单独声明，靠 `grep-ignore` 传递即可。）

- [ ] **Step 2: 装依赖，确认编译**

```bash
cargo build -p dozer-app 2>&1 | tail -30
```

- [ ] **Step 3: 新增图标资源**

创建 `crates/dozer-app/assets/icons/search.svg`（Lucide `search`，去掉 license 注释与
`class` 属性）：

```svg
<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
>
  <circle cx="11" cy="11" r="8" />
  <path d="m21 21-4.3-4.3" />
</svg>
```

`IconKind` 枚举加（挨着其它面板专属图标那一片）：

```rust
    /// 文件树右键"搜索"入口图标(Lucide search)。
    Search,
```

`bytes()` 的 `match` 里加：

```rust
            IconKind::Search => include_bytes!("../assets/icons/search.svg"),
```

- [ ] **Step 4: 编译/lint/格式确认**

```bash
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期：`IconKind::Search` 这一步还没被构造，有 `dead_code` 警告，Task 4 解决。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/Cargo.toml Cargo.lock crates/dozer-app/assets/icons/search.svg crates/dozer-app/src/icons.rs
git commit -m "feat(dozer-app): add grep dep and search icon"
```

---

### Task 2: 数据模型 + `search_scope` 纯函数 + 单测

**Files:**
- Create: `crates/dozer-app/src/extensions/search.rs`
- Modify: `crates/dozer-app/src/extensions.rs`（加 `pub mod search;`）

**Interfaces:**
- Produces:
  - `pub enum Scope { Dir(PathBuf), File(PathBuf) }`
  - `pub struct SearchHit { pub path: PathBuf, pub line_no: u64, pub line_text: String }`
  - `pub fn search_scope(scope: &Scope, query: &str) -> Result<Vec<(String, Vec<SearchHit>)>, String>`

- [ ] **Step 1: 文件头 + 数据模型**

```rust
// crates/dozer-app/src/extensions/search.rs
//! 文件树右键"搜索"弹窗：作用域(目录子树/单文件)内的全文内容搜索。瞬态弹窗，
//! 不挂 `LeftView`/左侧图标栏，形制参考文件编辑弹层 `edit_modal`。不做搜索历史/
//! 索引/后台预扫描，见
//! `docs/superpowers/specs/2026-08-12-tree-search-in-context-menu-design.md`。

use std::path::{Path, PathBuf};

/// 搜索作用域：右键目标。
#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    Dir(PathBuf),   // 目录整棵子树
    File(PathBuf),  // 单文件
}

/// 一条命中。
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub path: PathBuf,
    pub line_no: u64,
    pub line_text: String,
}
```

- [ ] **Step 2: `search_scope` 纯函数**

```rust
/// 在 scope 内做字面子串（默认大小写不敏感）搜索，按文件分组返回，组内行号升序。
/// 返回 `Err(String)`，不 panic。隐私：Dir 尊重 `.gitignore`/hidden，二进制/非法
/// UTF-8 文件跳过。
///
/// APi 精确名按 `grep` crate 实际版本核实（见 Global Constraints）——下面是结构示意：
/// - `grep::regex::RegexMatcher`（`new_line_matcher` / 带 `MatchCase` 的大小写控制）
/// - `grep::searcher::SearcherBuilder`（`sink`、逐行回调）
/// - `grep::ignore::IgnoreBuilder` + `WalkBuilder`（Dir 递归 walk）
pub fn search_scope(scope: &Scope, query: &str) -> Result<Vec<(String, Vec<SearchHit>)>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    // ...
    Ok(groups)
}
```

写代码时先用一个隔离的小实验核实 `grep` 三条 API 线的真实名字与调用形态（建临时目录放
几个文本文件，分别试"Dir 递归命中"/"仅 File 命中"/".gitignore 忽略文件不命中"三种输入，
看实际 behavior），验证通过再定 `search_scope` 的实现；不要凭猜测把方法名写死。目标
行为不可漂移：文件命中、行号正确、忽略规则生效、空查询返回空。

- [ ] **Step 3: 单测**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn file(p: &str, content: &str) -> PathBuf {
        let p = std::path::Path::new(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
        p.to_path_buf()
    }

    #[test]
    fn empty_query_yields_no_results() {
        let dir = tempfile::tempdir().unwrap();
        let f = file(&dir.path().join("a.txt").display().to_string(), "hello world\n");
        let hits = search_scope(&Scope::File(f), "  ").unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn single_file_matches_with_line_no() {
        let dir = tempfile::tempdir().unwrap();
        let f = file(&dir.path().join("a.txt").display().to_string(), "first\nneedle here\nthird\n");
        let hits = search_scope(&Scope::File(f), "needle").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1[0].line_no, 2);
    }

    #[test]
    fn dir_scope_recurses_and_skips_large_ignored_files() {
        // 建被 .gitignore 忽略的文件 + 未被忽略的命中文件，断言只命中后者。
        let dir = tempfile::tempdir().unwrap();
        // ...
    }

    #[test]
    fn binary_utf8_file_does_not_panic() {
        // 写入含非法 UTF-8 字节的文件，断言不 panic、可正常返回。
        let dir = tempfile::tempdir().unwrap();
        // ...
    }
}
```

（若该文件组为空/某些行为依赖 `.gitignore` 语义，写代码时把断言按 `grep` 实际行为校准——
重点是"不 panic、忽略规则生效"这两条主行为不偏向。）

- [ ] **Step 4: 注册模块**

`extensions.rs` 的 `pub mod` 按字母序插 `pub mod search;`（`project` 与 `ssh` 之间，若其它
模块已落地顺序相应调整，字母序本身不变）。

- [ ] **Step 5: 编译/测试/lint/格式**

```bash
cargo test -p dozer-app search:: -- --test-threads=1
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/search.rs crates/dozer-app/src/extensions.rs
git commit -m "feat(dozer-app): add search data model and scope-search logic"
```

---

### Task 3: `Message` + `WorkspaceState` + `update` + 弹窗 `view`

**Files:**
- Modify: `crates/dozer-app/src/extensions/search.rs`

**Interfaces:**
- Consumes:Task 2 的 `Scope`/`SearchHit`/`search_scope`。
- Produces:
  - `pub struct WorkspaceState`（`open`/`scope`/`query`/`query_editing`/`running`/
    `results`/`has_searched`/`error`）
  - `pub enum Message`
  - `pub fn open(ws: &mut WorkspaceState, scope: Scope)`
  - `pub fn update(ws: &mut WorkspaceState, msg: Message, project_id: i64, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`
  - `pub fn search_modal<'a>(ws: &'a WorkspaceState) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>`

- [ ] **Step 1: `WorkspaceState` + 存取方法**

```rust
#[derive(Default)]
pub struct WorkspaceState {
    open: bool,
    scope: Option<Scope>,
    query: String,
    query_editing: bool,
    running: bool,
    /// 已提交并回灌的命中结果。
    results: Vec<(String, Vec<SearchHit>)>,
    /// 区分"未搜索"与"搜索过但零命中"。
    has_searched: bool,
    error: Option<String>,
}

impl WorkspaceState {
    pub fn is_open(&self) -> bool { self.open }
    pub fn scope(&self) -> Option<&Scope> { self.scope.as_ref() }
    pub fn query(&self) -> &str { &self.query }
    pub fn query_editing(&self) -> bool { self.query_editing }
    pub fn results(&self) -> &[(String, Vec<SearchHit>)] { &self.results }
    pub fn has_searched(&self) -> bool { self.has_searched }
    pub fn error(&self) -> Option<&str> { self.error.as_deref() }
}
```

- [ ] **Step 2: `Message` 枚举**

```rust
#[derive(Debug, Clone)]
pub enum Message {
    SearchOpen(Scope),
    SearchClose,
    QueryChanged(String),
    QueryEditing(bool),
    QuerySubmit,
    SearchResults(i64, Result<Vec<(String, Vec<SearchHit>)>, String>),
    /// 点击命中 → 内核拦截映射为 `PreviewOpenPath`（本模块只声明，不进 update）。
    Pick(SearchHit),
}
```

- [ ] **Step 3: `update`**

```rust
pub fn open(ws: &mut WorkspaceState, scope: Scope) {
    ws.open = true;
    ws.scope = Some(scope);
    ws.error = None;
    ws.has_searched = false;
    ws.results.clear();
    // query/query_editing 保留上次，便于同项目内复用关键字。
}

pub fn update(
    ws: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::SearchOpen(scope) => open(ws, scope),
        Message::SearchClose => ws.open = false,
        Message::QueryChanged(s) => ws.query = s,
        Message::QueryEditing(b) => ws.query_editing = b,
        Message::QuerySubmit => {
            let Some(scope) = ws.scope.clone() else { return };
            let query = ws.query.clone();
            if query.trim().is_empty() { return; }
            ws.running = true;
            ws.error = None;
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || search_scope(&scope, &query))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                emit(Message::SearchResults(project_id, result));
            });
        }
        Message::SearchResults(_pid, result) => {
            ws.running = false;
            match result {
                Ok(results) => {
                    ws.results = results;
                    ws.has_searched = true;
                }
                Err(e) => ws.error = Some(e),
            }
        }
        Message::Pick(_) | Message::QueryEditing(_) => { /* 见注释，不落地 */ }
    }
}
```

（`QueryEditing` 其实不一定要单独变体——`query_editing` 由 `main.rs` 在进入/离开输入焦点
时通过内联 `search_modal` 的 view 消息或独立开关维护；若照 files 搜索框那套（
`FilesSearchEditStart`/`FilesSearchEvent`）能更省事，就照那套，别硬套这个枚举。写代码时
以最小改动、复用既有模式为准。）

- [ ] **Step 4: 弹窗 `search_modal`**

形制照抄 `workspace.rs::edit_modal`：满载 `SCRIM` 遮罩 + `CARD` 对话框。标题行 = 作用域名
（`Dir` 显示目录相对路径 + "※目录";`File` 显示文件名）+ `×`；正文 = 自绘关键字输入框
（展示态显示关键字 + 点击进入编辑、编辑态显示草稿，风格同 files 顶栏搜索框）+ "搜索"
按钮 + 结果 `Scrollable`（组标题 `相对路径`，组内每条 `:行号: 命中行`）+ 底部错误行：

```rust
pub fn search_modal<'a>(
    ws: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !ws.open {
        return column![].into();
    }
    let scope_label = match &ws.scope {
        Some(Scope::Dir(p)) => format!("目录: {}", p.display()),
        Some(Scope::File(p)) => format!("文件: {}", p.display()),
        None => String::new(),
    };
    let title_row = row![
        text(scope_label).size(theme::font::subtitle()).color(theme::color::CREAM),
        iced_widget::space::horizontal(),
        button(text("×").size(theme::font::subtitle()).color(theme::color::DIM))
            .on_press(Message::SearchClose).padding(0)
            .style(|_t, _s| button::Style {
                background: None,
                text_color: theme::color::DIM,
                ..button::Style::default()
            }),
    ].align_y(iced_widget::core::Alignment::Center);

    // 查询行：自绘输入框(见 files 的 framebox/nibox 模式) + "搜索"按钮
    let mut body = column![title_row, query_row].spacing(8);

    if ws.has_searched && ws.results.is_empty() && ws.error.is_none() {
        body = body.push(text("无匹配").size(theme::font::body()).color(theme::color::DIM));
    } else if !ws.results.is_empty() {
        body = body.push(results_list(ws));
    }
    if let Some(err) = &ws.error {
        body = body.push(text(format!("⚠ {err}"))
            .size(theme::font::body()).color(theme::color::RED));
    }

    let dialog = container(body.padding(16))
        .width(Length::Fill).height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::CARD.into()),
            border: Border { color: theme::color::BORDER, width: 1.0, radius: 6.0.into() },
            ..container::Style::default()
        });
    container(dialog)
        .padding(40.0).width(Length::Fill).height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::SCRIM.into()),
            ..container::Style::default()
        })
        .into()
}
```

`results_list` 用一个 `Scrollable` 列：每个命中项做成 `button`，`on_press(Message::Pick(hit))`，
显示 `相对路径:行号: 命中行`（文件分组标题是 `path.strip_prefix(project_root)` 后的相对
路径；行号与命中行文本用同色 CREAM，命中行可截断长文本）。

- [ ] **Step 5: 编译/lint/格式**

```bash
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期：仍可能 `dead_code`（`search_modal` 还没被 `App::view` 调用），Task 4 解决。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/search.rs
git commit -m "feat(dozer-app): add search popup state, update, and view"
```

---

### Task 4: 接入内核（右键菜单项 + 弹窗浮层 + 键盘路由）

**Files:**
- Modify: `crates/dozer-app/src/extensions/files.rs`（右键菜单加"搜索"项）
- Modify: `crates/dozer-app/src/app.rs`（`App::view` 浮层链加 `search_modal`；内核处理
  `SearchInContextMenu` 开弹窗、`Pick` 映射预览）
- Modify: `crates/dozer-app/src/main.rs`（拦截 `SearchInContextMenu`/`Pick`/键盘路由）
- Modify: `crates/dozer-app/src/workspace.rs`（`Workspace` struct 加 `search` 字段；若有
  `edit_modal` 那层的浮层链也在 app.rs，视符号分布调整）

**Interfaces:**
- Consumes:Task 2-3 的 `search::{WorkspaceState, Message, update, open, search_modal}`。

- [ ] **Step 1: `Workspace` 加字段**

`Workspace` struct 加：

```rust
search: search::WorkspaceState,
```

（若 `Workspace` 拆到 `workspace.rs`，就在对应文件加。）对应初始化处加
`search: search::WorkspaceState::default(),`。不需要改 `App` struct（跨项目各自保留，无
全局单例概念）。

- [ ] **Step 2: `files` 右键菜单加"搜索"项（最顶）**

`context_menu_popup` 的 `let mut items: Vec<...> = Vec::new();` 之后、`if menu.is_dir {`
（新建文件/新建文件夹）**之前**插入：

```rust
items.push(menu_item(
    Some(icons::IconKind::Search),
    "搜索",
    Message::SearchInContextMenu {
        path: menu.target.clone(),
        is_dir: menu.is_dir,
    },
));
push_sep(&mut items);
```

（目录、文件行都显示"搜索"。用 `is_dir` 拼 `Scope`。）

- [ ] **Step 3: `Message` 加变体 + 内核分发**

`files::Message` 加（只作拦截锚点，不进 `files::update`，同 `CopyPath` 的既有拦截模式）：

```rust
/// 右键菜单"搜索"：内核拦截，关菜单并打开聚焦 Workspace 上的搜索弹窗。
SearchInContextMenu { path: PathBuf, is_dir: bool },
```

顶层 `Message`（`app.rs`/`workspace.rs` 顶层）加：

```rust
Search(search::Message),
```

`App::update` 加分发（路由要求同数据库/SSH 面板：带显式 `project_id` 的异步结果
`SearchResults` 用 `with_project`，其余用 `with_focused_project`；特化分支排在通配
`Search(msg)` **之前**）：

```rust
Message::Search(search::Message::SearchResults(project_id, result)) => {
    self.with_project(project_id, move |ws, io| {
        search::update(&mut ws.search, search::Message::SearchResults(project_id, result),
            project_id, &io.handle,
            { let p = io.proxy.clone(); move |m| { let _ = p.send_event(Message::Search(m)); } });
    });
}
Message::Search(search::Message::SearchClose)
| Message::Search(search::Message::QueryChanged(_))
| Message::Search(search::Message::QueryEditing(_))
| Message::Search(search::Message::QuerySubmit) => {
    self.with_focused_project(|ws, io| {
        search::update(&mut ws.search, msg, /* project_id */ 0, &io.handle,
            { let p = io.proxy.clone(); move |m| { let _ = p.send_event(Message::Search(m)); } });
    });
}
```

（`with_project`/`with_focused_project`/`io.handle`/`io.proxy` 的确切签相照抄 `browser.rs`
里 `Message::Browser(msg) => { self.with_focused_project(|ws, io| { .. })}` 的真实写法，本
文档是形状示意。）

- [ ] **Step 4: 拦截 `SearchInContextMenu` 开弹窗**

`main.rs` 的 `dispatch()`（或 `CopyPath` 所在的既有拦截点附近）加：

```rust
Message::FilesSearchInContextMenu { path, is_dir } => {
    workspace.update(Message::FilesContextMenuClose);   // 关右键菜单(互斥清理)
    let scope = if is_dir { search::Scope::Dir(path) } else { search::Scope::File(path) };
    workspace.update(Message::Search(search::Message::SearchOpen(scope)));
}
```

`search::Message::SearchOpen` 走通配 `with_focused_project` 分支 → `open` 置
`open=true` + `scope`。（`context_menu` 关闭也可走既有 `FilesContextMenuClose` 变体；命名
以实际既有变体为准。）

- [ ] **Step 5: 拦截 `Pick` 打开预览**

`main.rs` 拦截 `Message::Search(search::Message::Pick(hit))`，映射为既有
`Message::PreviewOpenPath { path: hit.path }`（复刻文件树单击文件行那条既有拦截由
`files::Message::OpenFile` → `PreviewOpenPath` 的同一路径），并关弹窗：

```rust
Message::Search(search::Message::Pick(hit)) => {
    workspace.update(Message::Search(search::Message::SearchClose));
    // 打开该文件预览(预览打开时不保持弹窗悬浮,同互斥清理口径)
    workspace.update(Message::PreviewOpenPath { path: hit.path });
}
```

- [ ] **Step 6: 浮层链加 `search_modal`**

`App::view()` 顶层互斥浮层链的 `stack!`（`edit_modal`/`maximize_overlay` 所在层）插入
`search::search_modal(&ws.search).map(Message::Search)`，并在"该层当前该不该显示"的互斥
判断里把搜索弹窗算进去（同 `edit_modal` 打开时其它浮层给让位的判断链）。

- [ ] **Step 7: 键盘路由（`main.rs`）**

在既有键盘拦截层加两条（照抄 files 搜索框/编辑弹层那套口径）：
- `search.query_editing` 为真：按键路由成 `Message::Search(search::Message::QueryChanged(..))`,
  不再下钻 ⌘快捷键/地址栏/终端转发。
- 聚焦 Workspace 的 `search.is_open()` 且按 Esc：`Message::Search(search::Message::SearchClose)`
  （无脏状态，无需二次确认）。

- [ ] **Step 8: 编译/lint/格式收敛**

```bash
cargo build -p dozer-app 2>&1 | grep -E "^error"
```

反复跑，按报错补漏（常见：`use crate::extensions::search;` 没加进顶部批量
`use crate::extensions::{..};`）。

```bash
cargo build -p dozer-app
cargo test -p dozer-app -- --test-threads=1
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期：全部干净通过，零警告。

- [ ] **Step 9: Commit**

```bash
git add -u
git commit -m "feat(dozer-app): wire search popup into tree context menu and kernel"
```

---

### Task 5: 全量验证 + 人工验收

**Files:** 无新改动。

- [ ] **Step 1: 全量构建/测试/lint/格式**

```bash
cargo build
cargo test
cargo clippy --all-targets
cargo fmt --check
```

- [ ] **Step 2: 人工验收**

```bash
cargo run -p dozer-app
```

1. 打开一个项目，文件树某**目录**行右键：菜单**最顶**出现"搜索"（在"新建文件"之上）。
   点击 → 弹出搜索窗口（`SCRIM` 遮罩 + `CARD` 对话框），标题显示"目录: <相对路径>"，右键
   菜单已自动关闭。
2. 输入一个在若干该目录内文件文本中出现的单词，点"搜索"/回车 → 结果列表按文件分组、组内
   `:行号:` 命中行列出；结果里不应出现 `.gitignore` 忽略或隐藏的文件的命中行。
3. 点击某条结果 → 弹窗关闭，该文件在内核预览中打开。
4. 在某**文件**行右键"搜索"：标题显示"文件: <name>"，结果只来自该单个文件。
5. 输入不存在的词 → 显示"无匹配"；输纯空白不触发搜索。
6. 弹窗内输入框点击进入编辑态，敲字应只进搜索框、不落进聚焦终端；Esc / × / 点遮罩都能关
   闭弹窗。
7. 切到另一个项目再试：弹窗 scope/关键字按项目各自独立，不串。

- [ ] **Step 3: 确认分支状态**

```bash
git status
git log --oneline main..HEAD
```

按 Global Constraints 提请代码审阅，审阅通过后再合并——不在这个计划里自动合并。
