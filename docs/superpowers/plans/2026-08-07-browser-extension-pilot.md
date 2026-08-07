# 浏览器面板扩展化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把浏览器面板(tab/地址栏/收藏夹)拆成自洽模块 `extensions::browser`(自己的 `Message`/`State`/`update`/`view`),`workspace.rs` 内核只留包装转发——阶段 1 扩展化重构的第二个试点,验证 per-project 状态归属的范式(区别于 Git Log 试点的 App 级共享状态)。

**Architecture:** 新文件 `crates/dozer-app/src/extensions/browser.rs`:精简的 `Tabs` 类型(只认 URL,替代浏览器域对 `PreviewPane` 的复用)+ 收藏夹纯逻辑(从 `crate::bookmarks` 整体并入,该文件随后删除)+ `browser::Message`/`browser::State`/`browser::update`/`browser::request_bookmarks_refresh`/`browser::view`。`Workspace` 上 6 个 `browser*`/`bookmarks*` 字段合并成一个 `browser: extensions::browser::State`;顶层 `Message` 删 12 个 `Browser*` 变体,加 `Browser(extensions::browser::Message)`;内核 `update()` 三支(两个异步结果变体按 `ProjectId` 路由,其余点击驱动变体走 `with_focused_project` 统一转发)。`crate::preview::PreviewPane`/`WebviewSpec`、`crate::workspace::AddrEvent` 保持不动,由 `extensions::browser` 外部引用(前者是文件预览的共享基础设施,后者是地址栏/意见框/树编辑共用的通用文本输入事件类型,均不应移入浏览器专属模块)。

**Tech Stack:** Rust workspace;iced 0.14;`tokio::runtime::Handle::spawn` + 手工 `emit` 回调(同 Git Log 试点风格,不引入 `iced::Task`);`dozer_client::Client`(收藏夹 RPC)。

## Global Constraints

- 纯重构,不改变任何用户可见行为——收藏夹的全局/项目两级、乐观本地更新、去重规则原样保留;`Tabs::open_url` 不做去重(现有 `PreviewPane::open_url` 也没有,不顺手新增)。
- 不建 `Extension` trait/注册表,不拆独立 crate,不引入 `iced::Task` 风格异步返回值。
- `crate::preview::PreviewPane` 本身不动(文件预览"Files"核心面板继续用它);`extensions::browser` 用自己独立的 `Tabs` 类型,不复用不 fork 通用抽象。
- `crate::workspace::AddrEvent` 不移动(被浏览器地址栏/验收意见框/项目树行内编辑三处共用,真正的共享类型),`extensions::browser::Message::AddrEvent` 直接引用它。
- `crate::preview::WebviewSpec` 不移动(main.rs 的 webview 池合并 `preview_desired()`/`browser_desired()` 两路结果,必须是同一个类型),`Tabs::desired_webviews()` 返回它。
- 消息路由:单一 `browser::Message` 枚举,内核 `match` 特案 `BookmarksLoaded`/`BookmarksMutated`(按自带 `ProjectId` 走 `with_project`),其余统一走 `with_focused_project` + 通用转发——延续 Git Log 试点"内核特案跨面板知识、其余统一转发"的原则。
- 每个任务结束都要 `cargo build -p dozer-app && cargo test -p dozer-app` 干净通过。

---

### Task 1: 内核共享 UI 工具函数扩权限

浏览器视图要复用几个现在私有于 `workspace.rs`、且部分硬编码了顶层 `Message` 类型的 tab 栏
渲染工具函数。这个任务先把它们改成 `extensions::browser` 也能用的形状,不改变任何现有调用点
的行为。

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Produces: `pub(crate) fn tab_arrow_button<'a, M: 'a>(icon: icons::IconKind, enabled: bool, msg: M) -> Element<'a, M, iced_widget::Theme, iced_widget::Renderer>`、`pub(crate) fn tab_divider<'a, M: 'a>() -> Element<'a, M, iced_widget::Theme, iced_widget::Renderer>`、`pub(crate) fn lh<'a>(..) -> Text<'a>`(签名不变,只改可见性)、`pub(crate) fn preview_tab_display_width(title: &str) -> f32`、`pub(crate) fn tab_window(widths: &[f32], gap: f32, avail: f32, first: usize) -> (usize, bool, bool)`。

- [ ] **Step 1: `tab_arrow_button` 泛型化**

```rust
fn tab_arrow_button<'a>(
    icon: icons::IconKind,
    enabled: bool,
    msg: Message,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
```

改成:

```rust
pub(crate) fn tab_arrow_button<'a, M: 'a>(
    icon: icons::IconKind,
    enabled: bool,
    msg: M,
) -> Element<'a, M, iced_widget::Theme, iced_widget::Renderer> {
```

函数体不用改一个字(`btn.on_press(msg)` 本来就是对泛型 `Message` 参数类型工作,`iced_widget::button` 的 `on_press` 方法签名对任意 widget 消息类型都成立)。

- [ ] **Step 2: `tab_divider` 泛型化**

```rust
fn tab_divider<'a>() -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
```

改成:

```rust
pub(crate) fn tab_divider<'a, M: 'a>() -> Element<'a, M, iced_widget::Theme, iced_widget::Renderer> {
```

函数体不变(纯装饰性的 `Space`,从不构造 `Message` 值,泛型化零风险)。

- [ ] **Step 3: 其余三个函数改可见性**

```rust
fn lh<'a>(
```
→
```rust
pub(crate) fn lh<'a>(
```

```rust
fn preview_tab_display_width(title: &str) -> f32 {
```
→
```rust
pub(crate) fn preview_tab_display_width(title: &str) -> f32 {
```

```rust
fn tab_window(widths: &[f32], gap: f32, avail: f32, first: usize) -> (usize, bool, bool) {
```
→
```rust
pub(crate) fn tab_window(widths: &[f32], gap: f32, avail: f32, first: usize) -> (usize, bool, bool) {
```

- [ ] **Step 4: 编译 + 测试确认现有调用点不受影响**

Run: `cargo build -p dozer-app && cargo test -p dozer-app`
Expected: 干净通过——这一步只放宽可见性/泛型化两个纯 UI 函数,`workspace.rs` 内部所有现有
调用点(`tab_arrow_button(..., Message::XxxTabScroll(..))` 等)的类型推断应该自动落回
`M = Message`,不需要改动任何调用点代码。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(dozer-app): widen tab-bar UI helpers for cross-module reuse"
```

---

### Task 2: `extensions::browser` 的 `Tabs` 类型

**Files:**
- Create: `crates/dozer-app/src/extensions/browser.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(加 `pub mod browser;`)
- Modify: `crates/dozer-app/src/main.rs`(如 Task 1 未涉及,这里不需要改;`extensions` 模块
  声明已在 Git Log 试点里加过,不用重复加)

**Interfaces:**
- Produces:
  - `pub struct BrowserTab { pub id: usize, pub url: String, pub title: String }`
  - `pub struct Tabs`(私有字段:`tabs: Vec<BrowserTab>`, `active: usize`, `next_id: usize`,
    `addr_editing: bool`, `addr_buffer: String`;`#[derive(Default)]`)
  - `impl Tabs`:`tabs(&self) -> &[BrowserTab]`、`active_idx(&self) -> usize`、
    `open_url(&mut self, url: String) -> usize`、`select(&mut self, idx: usize)`、
    `close(&mut self, idx: usize)`、`addr_editing(&self) -> bool`、
    `addr_buffer(&self) -> &str`、`addr_begin(&mut self)`、`addr_text(&mut self, s: &str)`、
    `addr_backspace(&mut self)`、`addr_cancel(&mut self)`、
    `addr_submit(&mut self) -> Result<Option<String>, String>`、
    `active_webview_id(&self) -> Option<usize>`、
    `desired_webviews(&self) -> Vec<crate::preview::WebviewSpec>`

- [ ] **Step 1: 写失败的单测(先写整份文件的 `Tabs` 部分,含测试)**

创建 `crates/dozer-app/src/extensions/browser.rs`:

```rust
//! 浏览器面板扩展(阶段 1 扩展化重构第二个试点)。自己的 `Message`/
//! `State`/`update`/`view`,内核只认一个包装变体 `Message::Browser(..)`
//! 做转发,不知道自己被包在哪个外层类型里。收藏夹(全局/本项目两级书签)
//! 是浏览器域专属功能,状态/消息/UI 整体并入这个模块。
//!
//! `Tabs` 是浏览器专用的精简 tab/地址栏状态机,只认 URL——不复用
//! `crate::preview::PreviewPane`(文件预览专用,那边还有 `TabKind::File`/
//! `Acceptance`、`open_path`/`is_editable_extension`/`flyfish_url` 等浏览器
//! 用不到的逻辑),两者各自维护、互不知情。

use crate::preview::WebviewSpec;

/// 一个浏览器 tab。
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserTab {
    pub id: usize,
    pub url: String,
    pub title: String,
}

#[derive(Default)]
pub struct Tabs {
    tabs: Vec<BrowserTab>,
    active: usize,
    next_id: usize,
    addr_editing: bool,
    addr_buffer: String,
}

impl Tabs {
    pub fn tabs(&self) -> &[BrowserTab] {
        &self.tabs
    }

    pub fn active_idx(&self) -> usize {
        self.active
    }

    /// 取 URL 的 host 部分当标题(去掉协议头,取第一个 `/` 之前的部分)。
    /// 每次调用都新开一个 tab,不做去重——原样对齐现有
    /// `PreviewPane::open_url` 的行为(`open_path` 才有去重,`open_url`
    /// 没有),纯重构不改变这条现状。
    pub fn open_url(&mut self, url: String) -> usize {
        let title = url
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .split('/')
            .next()
            .unwrap_or(&url)
            .to_string();
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(BrowserTab {
            id,
            url,
            title,
        });
        self.active = self.tabs.len() - 1;
        id
    }

    pub fn select(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active = idx;
        }
    }

    pub fn close(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if idx < self.active {
            self.active -= 1;
        }
    }

    pub fn addr_editing(&self) -> bool {
        self.addr_editing
    }

    pub fn addr_buffer(&self) -> &str {
        &self.addr_buffer
    }

    /// 进入地址栏编辑:预填当前激活 tab 的 URL(浏览器 tab 恒为网页,不像
    /// `PreviewPane::addr_begin` 还要 match `TabKind`)。
    pub fn addr_begin(&mut self) {
        self.addr_editing = true;
        self.addr_buffer = self
            .tabs
            .get(self.active)
            .map(|t| t.url.clone())
            .unwrap_or_default();
    }

    pub fn addr_text(&mut self, s: &str) {
        self.addr_buffer.push_str(s);
    }

    pub fn addr_backspace(&mut self) {
        self.addr_buffer.pop();
    }

    pub fn addr_cancel(&mut self) {
        self.addr_editing = false;
        self.addr_buffer.clear();
    }

    /// 提交解析:`Ok(Some(url))` = 有效网址(无 scheme 自动补 `http://`);
    /// `Ok(None)` = 空输入,no-op;`Err(message)` = 本地路径(以 `/` 或
    /// `~/` 开头),浏览器不支持,`message` 是"浏览器不支持打开本地文件"
    /// 这条文案。解析规则原样照抄现有 `PreviewPane::addr_submit`/
    /// `AddrTarget` 那段逻辑,只是把"返回 `AddrTarget::File` 交给调用方
    /// 判断"改成直接在这里判定并通过 `Result` 表达。
    pub fn addr_submit(&mut self) -> Result<Option<String>, String> {
        self.addr_editing = false;
        let input = std::mem::take(&mut self.addr_buffer);
        let input = input.trim();
        if input.is_empty() {
            return Ok(None);
        }
        if input.starts_with('/') || input.starts_with("~/") {
            return Err("浏览器不支持打开本地文件".to_string());
        }
        if input.contains("://") {
            return Ok(Some(input.to_string()));
        }
        Ok(Some(format!("http://{input}")))
    }

    /// 当前激活 tab 的 webview id(浏览器 tab 恒为 webview,不像
    /// `PreviewPane::active_webview_id` 还要排除 `Acceptance`)。
    pub fn active_webview_id(&self) -> Option<usize> {
        self.tabs.get(self.active).map(|t| t.id)
    }

    /// webview 期望清单:每个 tab 一个,仅激活者可见。浏览器没有
    /// `PreviewPane` 那种"验收 tab 激活时其余全隐藏"的 overlay 概念,
    /// 也没有 flyfish 文件渲染 URL,直接拿 `url` 字段。
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| WebviewSpec {
                id: tab.id,
                url: tab.url.clone(),
                visible: idx == self.active,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_url_never_dedups_and_activates_new_tab() {
        let mut t = Tabs::default();
        let id0 = t.open_url("http://localhost:3000".into());
        let id1 = t.open_url("http://localhost:3000".into());
        assert_ne!(id0, id1, "每次都新开 tab,不去重");
        assert_eq!(t.tabs().len(), 2);
        assert_eq!(t.active_idx(), 1);
        assert_eq!(t.tabs()[0].title, "localhost:3000");
    }

    #[test]
    fn select_and_close_tabs() {
        let mut t = Tabs::default();
        t.open_url("http://a.com".into());
        t.open_url("http://b.com".into());
        t.select(0);
        assert_eq!(t.active_idx(), 0);
        t.close(0);
        assert_eq!(t.tabs().len(), 1);
        assert_eq!(t.active_idx(), 0);
        assert_eq!(t.tabs()[0].url, "http://b.com");
    }

    #[test]
    fn addr_edit_and_submit_parses_url_and_rejects_local_paths() {
        let mut t = Tabs::default();
        t.addr_begin();
        assert!(t.addr_editing());
        t.addr_text("localhost:3000/x");
        assert_eq!(t.addr_submit(), Ok(Some("http://localhost:3000/x".into())));
        assert!(!t.addr_editing());

        t.addr_begin();
        t.addr_text("https://example.com");
        assert_eq!(t.addr_submit(), Ok(Some("https://example.com".into())));

        t.addr_begin();
        t.addr_text("/tmp/x");
        assert_eq!(
            t.addr_submit(),
            Err("浏览器不支持打开本地文件".to_string())
        );

        t.addr_begin();
        t.addr_text("~/x");
        assert_eq!(
            t.addr_submit(),
            Err("浏览器不支持打开本地文件".to_string())
        );

        t.addr_begin();
        t.addr_text("abc");
        t.addr_backspace();
        t.addr_backspace();
        t.addr_backspace();
        assert_eq!(t.addr_submit(), Ok(None), "空输入不产生动作");

        t.addr_begin();
        t.addr_text("x");
        t.addr_cancel();
        assert!(!t.addr_editing());
    }

    #[test]
    fn addr_begin_prefills_current_tab_url() {
        let mut t = Tabs::default();
        t.open_url("http://a.com".into());
        t.addr_begin();
        assert_eq!(t.addr_buffer(), "http://a.com");
    }

    #[test]
    fn active_webview_id_tracks_active_tab() {
        let mut t = Tabs::default();
        assert_eq!(t.active_webview_id(), None, "无 tab 时没有 webview");
        let id0 = t.open_url("http://a.com".into());
        assert_eq!(t.active_webview_id(), Some(id0));
        let id1 = t.open_url("http://b.com".into());
        assert_eq!(t.active_webview_id(), Some(id1));
        t.select(0);
        assert_eq!(t.active_webview_id(), Some(id0));
    }

    #[test]
    fn desired_webviews_builds_urls_and_visibility() {
        let mut t = Tabs::default();
        t.open_url("http://a.com".into());
        t.open_url("http://b.com".into());
        let specs = t.desired_webviews();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].url, "http://a.com");
        assert!(!specs[0].visible, "非激活 tab 不可见");
        assert_eq!(specs[1].url, "http://b.com");
        assert!(specs[1].visible);
    }
}
```

- [ ] **Step 2: 声明模块**

`crates/dozer-app/src/extensions.rs`(Git Log 试点已建好这个文件,这里只加一行,按字母序插在
`git_log` 之前):

```rust
pub mod browser;
pub mod git_log;
```

- [ ] **Step 3: 运行测试确认通过**

Run: `cargo test -p dozer-app --bin dozer extensions::browser::`
Expected: 6 个测试全部 PASS。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions.rs crates/dozer-app/src/extensions/browser.rs
git commit -m "feat(dozer-app): add browser Tabs type (URL-only, no PreviewPane reuse)"
```

---

### Task 3: 收藏夹逻辑迁入 + `browser::Message`/`State`/`update`/`request_bookmarks_refresh`

**Files:**
- Modify: `crates/dozer-app/src/extensions/browser.rs`

**Interfaces:**
- Consumes: `Tabs`(Task 2)、`dozer_core::protocol::{BookmarkInfo, BookmarkScope}`、
  `dozer_client::Client`、`crate::workspace::AddrEvent`
- Produces:
  - `pub enum Message { OpenUrl(String), SelectTab(usize), CloseTab(usize), AddrClick, AddrEvent(crate::workspace::AddrEvent), TabScroll(bool), StarClick, BookmarkAdd(BookmarkScope), BookmarkRemove(i64), BookmarksToggle, BookmarksLoaded(i64, Vec<BookmarkInfo>), BookmarksMutated(i64, Result<(), String>) }`(`Debug, Clone`)
  - `pub struct State`(`Default`)
  - `impl State`:`addr_editing(&self) -> bool`、`addr_cancel(&mut self)`、
    `active_webview_id(&self) -> Option<usize>`、
    `desired_webviews(&self) -> Vec<crate::preview::WebviewSpec>`(均是对内部 `tabs` 字段的
    透传,内核用这些访问器,不直接碰 `State` 内部字段)
  - `pub fn update(state: &mut State, msg: Message, project_id: Option<i64>, client: &Client, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`
  - `pub fn request_bookmarks_refresh(project_id: Option<i64>, client: &Client, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`

- [ ] **Step 1: 把 `crate::bookmarks` 的纯逻辑并进来(改私有,不再 `pub`)**

在 `browser.rs` 顶部 `use crate::preview::WebviewSpec;` 之后追加:

```rust
use dozer_core::protocol::{BookmarkInfo, BookmarkScope};
```

在 `Tabs`/`impl Tabs`/其测试模块之后(`mod tests` 结束的 `}` 之后),插入(内容照抄现有
`crates/dozer-app/src/bookmarks.rs` 全文,只去掉每个 `pub` 关键字,因为现在只有本文件内部
的 `update`/`view` 会用到它们):

```rust
/// 乐观本地插入的占位 id:落库前不知道真实自增 id,只在"点击→下一次
/// 全量刷新落地"这一帧内部当哨兵用,不参与任何持久化或跨帧比较。
const OPTIMISTIC_BOOKMARK_ID: i64 = -1;

/// 当前 URL 在全局/本项目两边各自的收藏状态(`Some(id)` = 已收藏,
/// `id` 是"移出收藏"要传的记录 id)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BookmarkStatus {
    global: Option<i64>,
    project: Option<i64>,
}

impl BookmarkStatus {
    fn is_bookmarked(&self) -> bool {
        self.global.is_some() || self.project.is_some()
    }
}

fn bookmark_status(
    bookmarks: &[BookmarkInfo],
    url: &str,
    project_id: Option<i64>,
) -> BookmarkStatus {
    let global = bookmarks
        .iter()
        .find(|b| b.scope == BookmarkScope::Global && b.url == url)
        .map(|b| b.id);
    let project = project_id.and_then(|pid| {
        bookmarks
            .iter()
            .find(|b| b.scope == BookmarkScope::Project && b.project_id == Some(pid) && b.url == url)
            .map(|b| b.id)
    });
    BookmarkStatus { global, project }
}

/// 已存在(同 scope+project_id+url)则 no-op,幂等,与 dozerd 侧
/// `INSERT OR IGNORE` 语义一致。
fn optimistic_add(
    bookmarks: &mut Vec<BookmarkInfo>,
    scope: BookmarkScope,
    project_id: Option<i64>,
    url: &str,
    title: &str,
    created_ms: u64,
) {
    let exists = bookmarks
        .iter()
        .any(|b| b.scope == scope && b.project_id == project_id && b.url == url);
    if exists {
        return;
    }
    bookmarks.push(BookmarkInfo {
        id: OPTIMISTIC_BOOKMARK_ID,
        scope,
        project_id,
        url: url.to_string(),
        title: title.to_string(),
        created_ms,
    });
}

/// 按 id 过滤;未知 id 是 no-op,同 dozerd 侧 `remove` 语义。
fn optimistic_remove(bookmarks: &mut Vec<BookmarkInfo>, id: i64) {
    bookmarks.retain(|b| b.id != id);
}
```

紧跟着把现有 `crates/dozer-app/src/bookmarks.rs` 里 `mod tests` 的 8 个测试函数(`bookmark_status_*`/`is_bookmarked_*`/`optimistic_add_*`/`optimistic_remove_*`)原样搬进本文件已有的
`#[cfg(test)] mod tests { use super::*; ... }` 块里(跟 `Tabs` 的 6 个测试放在同一个
`mod tests` 里,不用重复写 `#[cfg(test)] mod tests`)。

- [ ] **Step 2: `browser::Message`**

紧跟 `optimistic_remove` 之后:

```rust
/// 浏览器面板自己的消息类型——内核(`workspace.rs`)只认一个包装变体
/// `Message::Browser(extensions::browser::Message)`,这个模块本身不
/// import 顶层 `Message`。`AddrEvent` 是地址栏/验收意见框/项目树行内
/// 编辑三处共用的通用文本输入事件类型,定义在 `crate::workspace`,这里
/// 直接引用,不复制。
#[derive(Debug, Clone)]
pub enum Message {
    OpenUrl(String),
    SelectTab(usize),
    CloseTab(usize),
    AddrClick,
    AddrEvent(crate::workspace::AddrEvent),
    TabScroll(bool),
    StarClick,
    BookmarkAdd(BookmarkScope),
    BookmarkRemove(i64),
    BookmarksToggle,
    BookmarksLoaded(i64, Vec<BookmarkInfo>),
    BookmarksMutated(i64, Result<(), String>),
}
```

- [ ] **Step 3: `browser::State`**

```rust
/// 浏览器面板的全部状态。挂在每个 `Workspace` 上(不像 Git Log 挂在
/// `App` 上)——项目切换靠 `Workspace` 自身生命周期天然隔离,不需要
/// 手动同步/清空逻辑。
#[derive(Default)]
pub struct State {
    tabs: Tabs,
    error: Option<String>,
    tab_first: usize,
    bookmarks: Vec<BookmarkInfo>,
    bookmarks_open: bool,
    star_menu_open: bool,
}

impl State {
    /// 地址栏是否在编辑态(内核 `App::browser_addr_editing` 键盘路由用)。
    pub fn addr_editing(&self) -> bool {
        self.tabs.addr_editing()
    }

    /// 取消地址栏编辑(内核 `App::blur_inputs` 用)。
    pub fn addr_cancel(&mut self) {
        self.tabs.addr_cancel();
    }

    /// 当前激活 tab 的 webview id(内核 `App::active_browser_webview_id`
    /// 焦点路由用)。
    pub fn active_webview_id(&self) -> Option<usize> {
        self.tabs.active_webview_id()
    }

    /// webview 期望清单(内核 `App::browser_desired` 用,供 main.rs 同步
    /// webview 池)。
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        self.tabs.desired_webviews()
    }
}
```

- [ ] **Step 4: `update` 函数**

```rust
/// 处理浏览器面板的全部消息。`project_id` 由内核每次调用时从
/// `ws.project.as_ref().map(|p| p.id)` 现取传入(`State` 本身不存这个,
/// 避免状态冗余/漂移)。`client` 用于收藏夹的 dozerd RPC 往返——这是跟
/// Git Log 试点 `update` 签名的主要差异(Git Log 只有本地 `git2` 阻塞
/// 调用,不需要网络)。
pub fn update(
    state: &mut State,
    msg: Message,
    project_id: Option<i64>,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::OpenUrl(url) => {
            state.error = None;
            state.tabs.open_url(url);
            state.tab_first = 0;
        }
        Message::SelectTab(idx) => state.tabs.select(idx),
        Message::CloseTab(idx) => {
            state.tabs.close(idx);
            state.tab_first = 0;
        }
        Message::AddrClick => {
            state.error = None;
            state.tabs.addr_begin();
        }
        Message::AddrEvent(ev) => match ev {
            crate::workspace::AddrEvent::Text(s) => state.tabs.addr_text(&s),
            crate::workspace::AddrEvent::Backspace => state.tabs.addr_backspace(),
            crate::workspace::AddrEvent::Cancel => state.tabs.addr_cancel(),
            crate::workspace::AddrEvent::Submit => match state.tabs.addr_submit() {
                Ok(Some(url)) => {
                    update(state, Message::OpenUrl(url), project_id, client, handle, emit)
                }
                Ok(None) => {}
                Err(message) => state.error = Some(message),
            },
        },
        Message::TabScroll(right) => {
            if right {
                state.tab_first = state.tab_first.saturating_add(2);
            } else {
                state.tab_first = state.tab_first.saturating_sub(2);
            }
        }
        Message::StarClick => {
            state.error = None;
            state.star_menu_open = !state.star_menu_open;
        }
        Message::BookmarksToggle => state.bookmarks_open = !state.bookmarks_open,
        Message::BookmarkAdd(scope) => {
            state.star_menu_open = false;
            let Some(tab) = state.tabs.tabs().get(state.tabs.active_idx()) else {
                return;
            };
            let url = tab.url.clone();
            let title = tab.title.clone();
            let target_project_id = match scope {
                BookmarkScope::Global => None,
                BookmarkScope::Project => project_id,
            };
            let created_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            optimistic_add(
                &mut state.bookmarks,
                scope,
                target_project_id,
                &url,
                &title,
                created_ms,
            );
            let Some(project_id) = project_id else { return };
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .add_bookmark(scope, target_project_id, &url, &title)
                    .await
                    .map_err(|e| e.to_string());
                emit(Message::BookmarksMutated(project_id, res));
            });
        }
        Message::BookmarkRemove(id) => {
            state.star_menu_open = false;
            optimistic_remove(&mut state.bookmarks, id);
            let Some(project_id) = project_id else { return };
            let client = client.clone();
            handle.spawn(async move {
                let res = client.remove_bookmark(id).await.map_err(|e| e.to_string());
                emit(Message::BookmarksMutated(project_id, res));
            });
        }
        Message::BookmarksLoaded(_, bookmarks) => state.bookmarks = bookmarks,
        Message::BookmarksMutated(_, res) => {
            if let Err(message) = res {
                state.error = Some(message);
            }
            request_bookmarks_refresh(project_id, client, handle, emit);
        }
    }
}
```

- [ ] **Step 5: `request_bookmarks_refresh` 函数**

紧跟 `update` 之后:

```rust
/// 现有 `Workspace::spawn_bookmarks_refresh` 的搬家版本:异步拉取
/// "全局 + 当前项目"收藏夹合集。
pub fn request_bookmarks_refresh(
    project_id: Option<i64>,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let Some(project_id) = project_id else { return };
    let client = client.clone();
    handle.spawn(async move {
        let bookmarks = client
            .list_bookmarks(Some(project_id))
            .await
            .unwrap_or_default();
        emit(Message::BookmarksLoaded(project_id, bookmarks));
    });
}
```

文件顶部再加一行 import:

```rust
use dozer_client::Client;
```

- [ ] **Step 6: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | head -100`
Expected: `extensions/browser.rs` 这个文件本身不应再报"未定义"类错误;`workspace.rs` 那边
大概率还报错(还在用旧的 12 个 `Browser*` 顶层变体、旧的 `crate::bookmarks` 模块),留给
Task 5 修。可用 `cargo check -p dozer-app 2>&1 | grep "extensions/browser.rs"` 单独确认
本文件没有报错。

- [ ] **Step 7: 新增 `update`/`request_bookmarks_refresh` 的单测**

在 `mod tests` 里追加(`use super::*;` 已存在):

```rust
    fn client_for_test() -> Client {
        Client::new(std::path::PathBuf::from("/tmp/dozer-browser-test-nonexistent.sock"))
    }

    fn bm(id: i64, scope: BookmarkScope, project_id: Option<i64>, url: &str) -> BookmarkInfo {
        BookmarkInfo {
            id,
            scope,
            project_id,
            url: url.into(),
            title: url.into(),
            created_ms: 0,
        }
    }

    #[test]
    fn bookmark_status_detects_global_and_project_independently() {
        let list = vec![
            bm(1, BookmarkScope::Global, None, "https://a.com"),
            bm(2, BookmarkScope::Project, Some(7), "https://a.com"),
        ];
        let status = bookmark_status(&list, "https://a.com", Some(7));
        assert_eq!(status.global, Some(1));
        assert_eq!(status.project, Some(2));
        assert!(status.is_bookmarked());
    }

    #[test]
    fn optimistic_add_is_idempotent_for_same_scope_and_url() {
        let mut list = vec![];
        optimistic_add(&mut list, BookmarkScope::Global, None, "https://a.com", "A", 100);
        optimistic_add(&mut list, BookmarkScope::Global, None, "https://a.com", "改名", 200);
        assert_eq!(list.len(), 1, "已存在则不重复插入");
        assert_eq!(list[0].title, "A");
    }

    #[tokio::test]
    async fn update_open_url_clears_error_and_resets_tab_first() {
        let mut state = State {
            error: Some("旧错误".to_string()),
            tab_first: 3,
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::OpenUrl("http://a.com".into()),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert!(state.error.is_none());
        assert_eq!(state.tab_first, 0);
        assert_eq!(state.tabs.tabs().len(), 1);
    }

    #[tokio::test]
    async fn update_addr_event_submit_ok_recurses_into_open_url() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(&mut state, Message::AddrClick, Some(1), &client, &handle, |_| {});
        update(
            &mut state,
            Message::AddrEvent(crate::workspace::AddrEvent::Text("http://a.com".into())),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        update(
            &mut state,
            Message::AddrEvent(crate::workspace::AddrEvent::Submit),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.tabs.tabs().len(), 1, "提交应递归触发 OpenUrl 开一个 tab");
        assert_eq!(state.tabs.tabs()[0].url, "http://a.com");
    }

    #[tokio::test]
    async fn update_addr_event_submit_local_path_sets_error_without_opening_tab() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(&mut state, Message::AddrClick, Some(1), &client, &handle, |_| {});
        update(
            &mut state,
            Message::AddrEvent(crate::workspace::AddrEvent::Text("/tmp/x".into())),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        update(
            &mut state,
            Message::AddrEvent(crate::workspace::AddrEvent::Submit),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.error.as_deref(), Some("浏览器不支持打开本地文件"));
        assert!(state.tabs.tabs().is_empty());
    }

    #[tokio::test]
    async fn update_tab_scroll_saturates_at_zero() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(&mut state, Message::TabScroll(false), Some(1), &client, &handle, |_| {});
        assert_eq!(state.tab_first, 0, "不该下溢");
        update(&mut state, Message::TabScroll(true), Some(1), &client, &handle, |_| {});
        assert_eq!(state.tab_first, 2);
    }

    #[tokio::test]
    async fn update_star_click_toggles_menu_and_clears_error() {
        let mut state = State {
            error: Some("旧错误".to_string()),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(&mut state, Message::StarClick, Some(1), &client, &handle, |_| {});
        assert!(state.star_menu_open);
        assert!(state.error.is_none());
        update(&mut state, Message::StarClick, Some(1), &client, &handle, |_| {});
        assert!(!state.star_menu_open);
    }

    #[tokio::test]
    async fn update_bookmarks_toggle_flips_panel() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(&mut state, Message::BookmarksToggle, Some(1), &client, &handle, |_| {});
        assert!(state.bookmarks_open);
    }

    #[tokio::test]
    async fn update_bookmark_add_without_active_tab_is_noop() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::BookmarkAdd(BookmarkScope::Global),
            Some(1),
            &client,
            &handle,
            |_| panic!("无 tab 时不该 emit"),
        );
        assert!(state.bookmarks.is_empty());
    }

    #[tokio::test]
    async fn update_bookmark_add_global_optimistically_inserts() {
        let mut state = State::default();
        state.tabs.open_url("http://a.com".into());
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::BookmarkAdd(BookmarkScope::Global),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.bookmarks.len(), 1);
        assert_eq!(state.bookmarks[0].scope, BookmarkScope::Global);
        assert_eq!(state.bookmarks[0].project_id, None);
        assert!(!state.star_menu_open);
    }

    #[tokio::test]
    async fn update_bookmark_add_project_scope_without_project_id_is_local_only() {
        let mut state = State::default();
        state.tabs.open_url("http://a.com".into());
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::BookmarkAdd(BookmarkScope::Project),
            None,
            &client,
            &handle,
            |_| panic!("project_id 为 None 时不该 emit 网络请求"),
        );
        assert_eq!(state.bookmarks.len(), 1, "本地乐观更新仍然发生");
    }

    #[tokio::test]
    async fn update_bookmark_remove_filters_local_cache() {
        let mut state = State {
            bookmarks: vec![bm(1, BookmarkScope::Global, None, "https://a.com")],
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::BookmarkRemove(1),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert!(state.bookmarks.is_empty());
    }

    #[tokio::test]
    async fn update_bookmarks_loaded_replaces_local_cache() {
        let mut state = State {
            bookmarks: vec![bm(1, BookmarkScope::Global, None, "https://stale.com")],
            ..State::default()
        };
        let fresh = vec![bm(2, BookmarkScope::Global, None, "https://fresh.com")];
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::BookmarksLoaded(1, fresh.clone()),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.bookmarks, fresh);
    }

    #[tokio::test]
    async fn update_bookmarks_mutated_error_sets_state_error() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::BookmarksMutated(1, Err("boom".to_string())),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.error.as_deref(), Some("boom"));
    }

    #[test]
    fn state_accessors_delegate_to_tabs() {
        let mut state = State::default();
        assert!(!state.addr_editing());
        assert_eq!(state.active_webview_id(), None);
        assert!(state.desired_webviews().is_empty());
        state.tabs.open_url("http://a.com".into());
        state.tabs.addr_begin();
        assert!(state.addr_editing());
        state.addr_cancel();
        assert!(!state.addr_editing());
        assert_eq!(state.active_webview_id(), Some(0));
        assert_eq!(state.desired_webviews().len(), 1);
    }
```

（`update_bookmarks_mutated_error_sets_state_error` 这个测试里 `BookmarksMutated` 分支会
调用 `request_bookmarks_refresh`,后者在 `project_id: Some(1)` 时会真的 `handle.spawn` 一个
连不上 `client_for_test()` 那个不存在 socket 的异步任务——`list_bookmarks` 会返回 `Err`,
`unwrap_or_default()` 落回空列表,`emit` 被调用但测试没有断言它,不影响本测试要验证的
`state.error` 这一步同步行为;`handle.spawn` 出去的任务在测试函数返回后可能还没跑完,这是
可接受的:同类模式在 Git Log 试点的 `request_refresh`/`update` 测试里已经用过。）

- [ ] **Step 8: 运行测试确认通过**

Run: `cargo test -p dozer-app --bin dozer extensions::browser:: 2>&1 | tail -80`
Expected: Task 2 的 6 个 + 本任务新增的 8 个收藏夹相关 + 14 个 `update`/`State` 相关测试,
共 28 个,全部 PASS。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/extensions/browser.rs
git commit -m "feat(dozer-app): fold bookmarks logic into browser::Message/State/update"
```

---

### Task 4: `browser::view` 与渲染辅助函数

**Files:**
- Modify: `crates/dozer-app/src/extensions/browser.rs`

**Interfaces:**
- Consumes: `Tabs`/`State`(Task 2/3)、`crate::workspace::{tab_arrow_button, tab_divider, lh, preview_tab_display_width, tab_window}`(Task 1,`pub(crate)` 后可跨模块调用)
- Produces: `pub fn view(state: &State, project_id: Option<i64>, width: Length, outer: Border) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>`

- [ ] **Step 1: 文件顶部补齐 `view` 需要的 import**

在 `browser.rs` 顶部现有 `use` 语句块里追加:

```rust
use crate::{chrome_style, icon_size, icons, theme, workspace_font, workspace_geometry};
use crate::workspace::{lh, preview_tab_display_width, tab_arrow_button, tab_divider};
use iced_widget::core::{Border, Element, Length};
use iced_widget::{button, column, container, row, text};
```

- [ ] **Step 2: `view` 主体(整体照搬现有 `browser_pane`,签名与类型改成本模块的)**

紧跟 `request_bookmarks_refresh` 之后:

```rust
/// 渲染整块浏览器面板:tab 栏 + 地址栏(+ 星标/收藏夹按钮)+(可能的)
/// 星标菜单/收藏面板 + 错误文案 + tab 内容或空态。`project_id` 由内核
/// 传入,用于收藏夹按项目分组渲染(`State` 本身不存这个)。`width`/`outer`
/// 是内核布局层算出的纯布局参数(圆角边框随左右面板区聚焦态变化,见
/// `zone_pane_border`)——现有 `browser_pane(ws, width, outer)` 就吃这两个
/// 参数,`terminal_pane`/`project_pane`/`preview_pane`/`todo_pane` 这些同
/// 级别的 `_pane` 函数也都是这个参数风格,保留它们、不是"图纯粹"把它们
/// 砍掉:砍掉会丢失焦点态圆角边框这条现有可见效果,违反"纯重构不改变
/// 可见行为"的约束。
pub fn view(
    state: &State,
    project_id: Option<i64>,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::browser_pane();
    let widths: Vec<f32> = state
        .tabs
        .tabs()
        .iter()
        .map(|t| preview_tab_display_width(&t.title))
        .collect();
    let (first, can_left, can_right) = tab_window(
        &widths,
        4.0,
        workspace_geometry::tab_bar_avail_px(),
        state.tab_first,
    );

    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> = state
        .tabs
        .tabs()
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx >= first)
        .map(|(idx, tab)| {
            let active = idx == state.tabs.active_idx();
            let select = button(lh(text(tab.title.clone())
                .size(workspace_font::subtitle())
                .color(theme::CREAM)))
            .on_press(Message::SelectTab(idx))
            .style(|_t, _s| button::Style {
                background: None,
                text_color: theme::CREAM,
                ..button::Style::default()
            });
            let close = button(lh(text("×").size(workspace_font::body()).color(theme::DIM)))
                .on_press(Message::CloseTab(idx))
                .style(|_t, _s| button::Style {
                    background: None,
                    text_color: theme::DIM,
                    ..button::Style::default()
                });
            container(
                row![select, close]
                    .spacing(2)
                    .align_y(iced_widget::core::Alignment::Center),
            )
            .padding([2, 4])
            .style(move |_t: &iced_widget::Theme| {
                if active {
                    container::Style {
                        background: Some(theme::CARD.into()),
                        border: Border {
                            color: theme::BORDER,
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..container::Style::default()
                    }
                } else {
                    container::Style::default()
                }
            })
            .into()
        })
        .collect();
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    let left_arrow = tab_arrow_button(icons::IconKind::ChevronLeft, can_left, Message::TabScroll(false));
    let right_arrow = tab_arrow_button(icons::IconKind::ChevronRight, can_right, Message::TabScroll(true));
    let tab_bar = row![left_arrow, right_arrow, clipped]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);

    let editing = state.tabs.addr_editing();
    let addr_text = if editing {
        format!("{}▏", state.tabs.addr_buffer())
    } else {
        "输入网址".to_string()
    };
    let addr = button(lh(text(addr_text)
        .size(workspace_font::body())
        .color(if editing { theme::CREAM } else { theme::DIM })))
    .on_press(Message::AddrClick)
    .width(Length::Fill)
    .style(move |_t, _s| button::Style {
        background: Some(theme::TERM_BG.into()),
        text_color: theme::CREAM,
        border: Border {
            color: if editing { theme::GOLD } else { theme::BORDER },
            width: 1.0,
            radius: 2.0.into(),
        },
        ..button::Style::default()
    });

    let addr_row = row![addr, star_button(state, project_id), bookmarks_toggle_button()]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);

    let mut content = column![tab_bar, tab_divider(), addr_row].spacing(region.gap);

    if state.star_menu_open {
        content = content.push(star_menu_popup(state, project_id));
    }
    if state.bookmarks_open {
        content = content.push(bookmarks_panel(state, project_id));
    }

    if let Some(err) = &state.error {
        content = content.push(lh(text(format!("⚠ {err}"))
            .size(workspace_font::body())
            .color(theme::RED)));
    }

    if state.tabs.tabs().is_empty() {
        content = content.push(
            container(lh(text("暂无网页——在地址栏输入网址")
                .size(workspace_font::subtitle())
                .color(theme::DIM)))
            .width(Length::Fill)
            .height(Length::Fill),
        );
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}
```

- [ ] **Step 3: 星标按钮/收藏夹按钮/菜单/面板辅助函数**

紧跟 `view` 之后:

```rust
/// 地址栏星标:当前 URL 在全局/本项目任一边已收藏则 GOLD 实心,否则
/// DIM;当前无 tab 时禁用(浏览器 tab 恒为网页,不需要像旧版
/// `current_browser_url` 那样再判一次 `TabKind`)。`project_id` 必须传
/// 真实值,不能传 `None` 占位——否则"只在本项目收藏、没加全局收藏"的
/// 网址会被误判成未收藏(`bookmark_status` 的 `project` 字段只在传了
/// `Some(pid)` 时才会去匹配)。
fn star_button(
    state: &State,
    project_id: Option<i64>,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let url = state
        .tabs
        .tabs()
        .get(state.tabs.active_idx())
        .map(|t| t.url.clone());
    let starred = url
        .as_ref()
        .map(|u| bookmark_status(&state.bookmarks, u, project_id).is_bookmarked())
        .unwrap_or(false);
    let color = if starred { theme::GOLD } else { theme::DIM };
    let mut btn = button(icons::view(icons::IconKind::Star, icon_size::row(), color))
        .width(Length::Fixed(workspace_geometry::tab_button_size()))
        .height(Length::Fixed(workspace_geometry::tab_button_size()))
        .padding(0)
        .style(move |_t, _s| button::Style {
            background: None,
            text_color: color,
            ..button::Style::default()
        });
    if url.is_some() {
        btn = btn.on_press(Message::StarClick);
    }
    btn.into()
}

/// tab 栏"收藏夹"下拉面板触发按钮,颜色恒定。
fn bookmarks_toggle_button<'a>() -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    button(icons::view(icons::IconKind::Bookmark, icon_size::row(), theme::DIM))
        .on_press(Message::BookmarksToggle)
        .width(Length::Fixed(workspace_geometry::tab_button_size()))
        .height(Length::Fixed(workspace_geometry::tab_button_size()))
        .padding(0)
        .style(|_t, _s| button::Style {
            background: None,
            text_color: theme::DIM,
            ..button::Style::default()
        })
        .into()
}

fn bookmark_menu_row(
    label: String,
    msg: Message,
) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    button(lh(text(label).size(workspace_font::body()).color(theme::CREAM)))
        .on_press(msg)
        .width(Length::Fill)
        .padding([6, 12])
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::CREAM,
            ..button::Style::default()
        })
        .into()
}

/// 星标小菜单:未收藏显示"加入…",已收藏显示"移出…"。
fn star_menu_popup(
    state: &State,
    project_id: Option<i64>,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(url) = state
        .tabs
        .tabs()
        .get(state.tabs.active_idx())
        .map(|t| t.url.clone())
    else {
        return column![].into();
    };
    let status = bookmark_status(&state.bookmarks, &url, project_id);

    let mut col = column![match status.global {
        Some(id) => bookmark_menu_row("移出全局收藏".to_string(), Message::BookmarkRemove(id)),
        None => bookmark_menu_row(
            "加入全局收藏".to_string(),
            Message::BookmarkAdd(BookmarkScope::Global)
        ),
    }]
    .spacing(2);

    if project_id.is_some() {
        col = col.push(match status.project {
            Some(id) => bookmark_menu_row("移出本项目收藏".to_string(), Message::BookmarkRemove(id)),
            None => bookmark_menu_row(
                "加入本项目收藏".to_string(),
                Message::BookmarkAdd(BookmarkScope::Project),
            ),
        });
    }

    container(col)
        .padding(6)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// 一组收藏条目:标题(点击新开 tab)+ `×` 删除按钮。
fn bookmark_group<'a>(
    title: &'static str,
    items: &[&'a BookmarkInfo],
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![lh(text(title).size(workspace_font::subtitle()).color(theme::DIM))]
        .spacing(2);
    for b in items {
        let open = button(lh(text(b.title.clone())
            .size(workspace_font::body())
            .color(theme::CREAM)))
        .on_press(Message::OpenUrl(b.url.clone()))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::CREAM,
            ..button::Style::default()
        });
        let remove = button(lh(text("×").size(workspace_font::body()).color(theme::DIM)))
            .on_press(Message::BookmarkRemove(b.id))
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: None,
                text_color: theme::DIM,
                ..button::Style::default()
            });
        col = col.push(
            row![open, remove]
                .spacing(4)
                .align_y(iced_widget::core::Alignment::Center),
        );
    }
    col.into()
}

/// 收藏夹下拉面板:分"全局收藏"/"本项目收藏"两组,都为空时显示占位文案。
fn bookmarks_panel(
    state: &State,
    project_id: Option<i64>,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let global: Vec<&BookmarkInfo> = state
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Global)
        .collect();
    let project: Vec<&BookmarkInfo> = state
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Project && b.project_id == project_id)
        .collect();

    let both_empty = global.is_empty() && project.is_empty();
    let mut col = column![].spacing(6);
    col = col.push(bookmark_group("全局收藏", &global));
    if project_id.is_some() {
        col = col.push(bookmark_group("本项目收藏", &project));
    }
    if both_empty {
        col = col.push(lh(text("暂无收藏")
            .size(workspace_font::subtitle())
            .color(theme::DIM)));
    }

    container(col)
        .padding(6)
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}
```

- [ ] **Step 4: 编译,逐条修正**

Run: `cargo build -p dozer-app 2>&1 | head -150`
Expected: 大概率会有 `chrome_style`/`icon_size`/`icons`/`theme`/`workspace_font`/
`workspace_geometry` 等 `use` 路径的细节问题,对照 `extensions/git_log.rs` 顶部已经验证过的
`use crate::theme;` 等写法修正。逐条修正直到 `cargo build -p dozer-app` 干净通过。

- [ ] **Step 5: `cargo test`/`clippy`/`fmt`**

Run: `cargo test -p dozer-app --bin dozer extensions::browser:: && cargo clippy -p dozer-app --all-targets && cargo fmt --check`
Expected: 全部通过。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/browser.rs
git commit -m "feat(dozer-app): add browser::view and bookmark UI render helpers"
```

---

### Task 5: 内核接线 + 清理旧代码

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/main.rs`
- Delete: `crates/dozer-app/src/bookmarks.rs`

**Interfaces:**
- Consumes: `extensions::browser::{Message, State, update, request_bookmarks_refresh, view}`
  (Task 2-4)

- [ ] **Step 1: `Workspace` 结构体字段合并**

把这 6 行(约第 1465-1479 行区域,连同各自文档注释)从:

```rust
    browser: PreviewPane,
    browser_error: Option<String>,
    bookmarks: Vec<BookmarkInfo>,
    browser_bookmarks_open: bool,
    browser_star_menu_open: bool,
```

（连同它们中间穿插的 `preview`/`preview_error` 不动,只删这 5 个 browser/bookmarks 相关
字段,`browser_tab_first`(约第 1523 行,在别的字段中间)也一并删除)替换成一个:

```rust
    /// 浏览器面板状态——自己的 `Message`/`update`/`view`,见
    /// `extensions::browser`。挂在每个 `Workspace` 上(不像 Git Log 挂在
    /// `App` 上),项目切换靠 `Workspace` 生命周期天然隔离。
    browser: browser::State,
```

（在文件顶部 `use crate::extensions::git_log;` 附近加一行 `use crate::extensions::browser;`。)

- [ ] **Step 2: `empty_for_project_placeholder()` 初始化**

原来的:

```rust
            browser: PreviewPane::default(),
            browser_error: None,
```

和(在别处的)

```rust
            browser_tab_first: 0,
            bookmarks: Vec::new(),
            browser_bookmarks_open: false,
            browser_star_menu_open: false,
```

全部删除,换成一行(位置在原 `browser: PreviewPane::default()` 那里):

```rust
            browser: browser::State::default(),
```

- [ ] **Step 3: 顶层 `Message` 枚举**

删除这 12 个变体(连同文档注释,分散在两处:大部分在 `BrowserAddrEvent` 附近一堆,
`BrowserTabScroll` 单独在 `PreviewTabScroll` 旁边,`BrowserBookmarksLoaded`/
`BrowserBookmarksMutated` 单独在 `AcceptanceCountLoaded` 旁边):

```rust
    BrowserTabScroll(bool),
    BrowserOpenUrl(String),
    BrowserSelectTab(usize),
    BrowserCloseTab(usize),
    BrowserAddrClick,
    BrowserAddrEvent(AddrEvent),
    BrowserStarClick,
    BrowserBookmarkAdd(BookmarkScope),
    BrowserBookmarkRemove(i64),
    BrowserBookmarksToggle,
    BrowserBookmarksLoaded(ProjectId, Vec<BookmarkInfo>),
    BrowserBookmarksMutated(ProjectId, Result<(), String>),
```

在原 `BrowserAddrEvent(AddrEvent)` 所在位置(紧跟 `PreviewEditConfirmCancel` 之后)加一个:

```rust
    /// 浏览器面板的全部消息,内核只转发不解读——见
    /// `extensions::browser::Message`。
    Browser(browser::Message),
```

- [ ] **Step 4: `update()` 里的分支**

删除以下这些旧分支(整段删掉):`Message::BrowserTabScroll`(在 `PreviewTabScroll` 分支
之后)、`Message::BrowserOpenUrl` 到 `Message::BrowserBookmarksMutated` 这一整串(共 11 个
分支,`AddrEvent`/`BookmarkAdd`/`BookmarkRemove` 内容较长)。

换成三支(位置放在原 `Message::BrowserOpenUrl` 所在的地方):

```rust
            Message::Browser(browser::Message::BookmarksLoaded(pid, bookmarks)) => {
                self.with_project(pid, move |ws, io| {
                    let handle = io.handle.clone();
                    let client = io.client.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Browser(m));
                    };
                    browser::update(
                        &mut ws.browser,
                        browser::Message::BookmarksLoaded(pid, bookmarks),
                        Some(pid),
                        &client,
                        &handle,
                        emit,
                    );
                });
            }
            Message::Browser(browser::Message::BookmarksMutated(pid, res)) => {
                self.with_project(pid, move |ws, io| {
                    let handle = io.handle.clone();
                    let client = io.client.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Browser(m));
                    };
                    browser::update(
                        &mut ws.browser,
                        browser::Message::BookmarksMutated(pid, res),
                        Some(pid),
                        &client,
                        &handle,
                        emit,
                    );
                });
            }
            Message::Browser(msg) => {
                self.with_focused_project(|ws, io| {
                    let project_id = ws.project.as_ref().map(|p| p.id);
                    let client = io.client.clone();
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Browser(m));
                    };
                    browser::update(&mut ws.browser, msg, project_id, &client, &handle, emit);
                });
            }
```

- [ ] **Step 5: 内核访问器方法改用 `browser::State` 的新接口**

`Workspace::browser_addr_editing`(约第 2684 行):

```rust
    pub fn browser_addr_editing(&self) -> bool {
        self.browser.addr_editing()
    }
```

不用改(`browser::State::addr_editing()` 签名跟 `PreviewPane::addr_editing()` 一样)。

`Workspace::active_browser_webview_id`(约第 2701 行)、`Workspace::blur_inputs` 里
`self.browser.addr_editing()`/`self.browser.addr_cancel()`(约第 2723-2724 行)、
`Workspace::browser_desired` 调用点里 `ws.browser.desired_webviews()`(约第 3508 行)—— 这
几处方法名跟 `browser::State` 暴露的访问器名字(`active_webview_id`/`addr_editing`/
`addr_cancel`/`desired_webviews`)完全一致,**不用改任何代码**,只是它们现在调用的是
`browser::State` 而不是 `PreviewPane` 的同名方法。

- [ ] **Step 6: `adopt_project`/`from_restore` 的刷新调用点**

原来的 `ws.spawn_bookmarks_refresh(io)`(约第 1773 行,`from_restore` 内)和
`self.spawn_bookmarks_refresh(io)`(约第 2307 行,`adopt_project` 内)改成:

```rust
        browser::request_bookmarks_refresh(
            ws.project.as_ref().map(|p| p.id),
            &io.client,
            &io.handle,
            {
                let proxy = io.proxy.clone();
                move |m| {
                    let _ = proxy.send_event(Message::Browser(m));
                }
            },
        );
```

（`from_restore` 内用 `ws.project`,`adopt_project` 内用 `self.project`——按各自函数体里
"当前操作的是哪个 `Workspace`"对应替换,不要都写 `ws.project` 或都写 `self.project`。)

同时删除 `Workspace::spawn_bookmarks_refresh` 这个方法本身(约第 2174-2186 行,已经被
`browser::request_bookmarks_refresh` 取代)。

- [ ] **Step 7: `browser_pane`/辅助渲染函数整段删除**

删除 `workspace.rs` 里的 `fn browser_pane`、`fn current_browser_url`、
`fn browser_star_button`、`fn browser_bookmarks_toggle_button`、`fn bookmark_menu_row`、
`fn browser_star_menu_popup`、`fn bookmark_group`、`fn browser_bookmarks_panel` 这 8 个
函数整段(它们的内容已经在 Task 4 里搬进 `extensions/browser.rs`)。

`App::view()` 里原来的:

```rust
        LeftView::Web => browser_pane(ws, Length::Fill, zone_pane_border(zone, ac)),
```

改成:

```rust
        LeftView::Web => browser::view(
            &ws.browser,
            ws.project.as_ref().map(|p| p.id),
            Length::Fill,
            zone_pane_border(zone, ac),
        )
        .map(Message::Browser),
```

- [ ] **Step 8: 删除 `use crate::bookmarks;` 与旧模块**

`workspace.rs` 顶部删除 `use crate::bookmarks;`(收藏夹逻辑已并入
`extensions::browser`,不再需要这个模块)。

`main.rs` 删除 `mod bookmarks;` 声明。

删除文件 `crates/dozer-app/src/bookmarks.rs`:

```bash
git rm crates/dozer-app/src/bookmarks.rs
```

- [ ] **Step 9: `main.rs` 里 `AddrEvent` 构造消息的那一行**

约第 699 行:

```rust
                    let message = if to_browser {
                        Message::BrowserAddrEvent(ev)
                    } else if to_comment {
```

改成:

```rust
                    let message = if to_browser {
                        Message::Browser(extensions::browser::Message::AddrEvent(ev))
                    } else if to_comment {
```

（`workspace::AddrEvent` 本身不动,`ev` 的类型不变,只是把"哪个顶层 `Message` 变体装它"
从 `BrowserAddrEvent(ev)` 换成 `Browser(extensions::browser::Message::AddrEvent(ev))`。
`main.rs` 已经在文件顶部 `mod extensions;`(Git Log 试点加的),不需要额外 `use`——照抄
main.rs 现有 `workspace::AddrEvent::Text(..)` 那种"`mod` 声明过的模块直接带路径引用,不
额外 `use` 单个类型"的风格,`extensions::browser::Message::AddrEvent` 同理直接可用。)

- [ ] **Step 10: 编译,逐条修正**

Run: `cargo build -p dozer-app 2>&1 | head -200`
Expected: 会有不少细节要修——按报错逐条核对是不是漏删了旧字段/旧分支、`with_project`/
`with_focused_project` 闭包里的变量捕获顺序、`browser::State` 各访问器名字是否对得上。
**不要**为了让它编译过而绕开"两个异步结果变体按 `ProjectId` 路由、其余走
`with_focused_project`"这条既定规则。

- [ ] **Step 11: 全量测试 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt --check`
Expected: 全绿。

- [ ] **Step 12: Commit**

```bash
git add -A crates/dozer-app
git commit -m "refactor(dozer-app): route browser messages through extensions::browser"
```

---

### Task 6: 全量校验与人工验收

**Files:** 无新增/修改(纯校验任务)

- [ ] **Step 1: 全 workspace 构建 + 测试 + clippy + fmt**

Run: `cargo build && cargo test && cargo clippy --all-targets && cargo fmt --check`
Expected: 全部 crate 编译通过、测试全绿、无警告、无格式差异。

- [ ] **Step 2: 人工验收(`cargo run -p dozer-app`)**

打开一个项目,点左图标栏"地球"进入浏览器:
1. 地址栏输入网址打开——新 tab 应出现,webview 应正常渲染网页。
2. 打开第二个网址,tab 切换/关闭应正常;关到 0 个 tab 时显示"暂无网页——在地址栏输入网址"。
3. 地址栏输入本地路径(如 `/tmp`)提交——应显示"浏览器不支持打开本地文件"红字,不开 tab。
4. 点星标"加入全局收藏"——星标变 GOLD 实心;点 tab 栏"收藏夹"图标——面板展开,"全局收藏"
   组里能看到这条,标题跟 tab 一致。
5. 点面板里的收藏条目——应新开 tab 打开同一网址。
6. 星标菜单/面板里点"移出"——条目消失,星标回到 DIM 空心。
7. 加一条"本项目收藏",切到另一个项目页签打开浏览器——"全局收藏"组应看到之前加的全局那条
   (跨项目共享),"本项目收藏"组应为空(项目隔离)。
8. 重启 `cargo run -p dozer-app`——收藏应该还在(验证真的落库,不是纯内存)。
9. 切换到"Files"面板(左图标栏),确认文件预览功能完全不受影响(打开文件、编辑保存等)——
   这是验证 `PreviewPane` 真的没被这次改动波及的关键一步。

若上述任一步与预期不符,对照 Task 3/4/5 的具体分支重新核对(尤其 Task 4 里提到的
`star_button` 需要 `project_id` 参数那个易错点)。

- [ ] **Step 3: 确认没有遗留未提交的改动**

Run: `git status`
Expected: 干净。
