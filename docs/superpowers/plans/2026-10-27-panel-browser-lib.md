# 浏览器面板抽成共享库 `dozer-panel-browser` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `crates/dozer-app/src/extensions/browser.rs`(2385 行:页签/地址栏状态机、收藏夹、`update`、`view`、文案)抽成共享库 `dozer-panel-browser`;Dozer 改为薄调用,**用户可见行为不变**。

**Architecture:** 与 `dozer-panel-todo` / `dozer-panel-project` 同模:新库依赖闭包不含 `dozer-app` / `dozer-client` / `dozer-core` / `wry` / `winit`;请求追踪用 `dozer-panel-host`,取数用 `dozer-services::BookmarkService`,产品相关的展示(页签控件、分隔条、区域样式)经 `BrowserDisplay` 适配 trait 注入。库只产出纯数据的 `BrowserWebviewSpec`;wry 池、IPC、前进/后退/刷新的真正执行、拖拽换位、地址栏焦点路由、收藏夹分割线几何留在 `dozer-app`(它们本来就在 `extensions/browser.rs` 之外)。Dozer 的 `extensions/browser.rs` 缩成"重新导出 + 少量适配",`app/` 里约 40 处 `browser::…` 调用点基本不动。**本 plan 不做 Digger 接入**(另起一期,模板是 S7/S8 系列)。

**Tech Stack:** Rust 2024 workspace、iced 0.14、byteui `v0.4.6`、bytehost `v0.4.12`(与 `dozer-panel-todo/Cargo.toml` 逐个一致)、`dozer-model` / `dozer-services` / `dozer-i18n` / `dozer-panel-host`。

**Spec:** 无独立 spec。抽库模式沿用 `docs/superpowers/specs/2026-10-15-panel-todo-lib-design.md`(§4 边界与适配接缝)与 `docs/superpowers/reviews/2026-10-26-s8a-report.md`(最近一次抽库的踩坑记录);浏览器相关约束见 `CLAUDE.md`「关键裁决」。本 plan 的边界决定都写在上面 Architecture 与下面 File Structure 里。

## Global Constraints

- **用户可见行为不变**:不改界面、交互、文案、协议、`dozerd`。搬走的测试**期望值不改、不删**;抽库前后测试总数核对按"基线 + 新增"算(Task 0 记基线,Task 6 对比)。
- **库依赖闭包不含** `dozer-app` / `dozer-client` / `dozer-core` / `dozerd` / `wry` / `winit`。验证:`cargo tree -p dozer-panel-browser | grep -E "dozer-(app|client|core)|dozerd|wry|winit"` 无输出。
- 不升级任何依赖;byteui / bytehost 钉版与 `crates/dozer-panel-todo/Cargo.toml` 一字不差(`bash scripts/check-shared-deps.sh` 会拦)。
- 库日志用 `tracing`,target 固定 `dozer::panel::web`(来源名必须是 `PanelKind::Web` 对应的面板名 `web`);不用 `dozer_core::log`。**不得**把地址栏输入/URL 之外的敏感内容写日志。
- 瞬时消息规则不变:库里只有持久的 `State.error`(内联展示,留在原位);库**不得**自己画 Toast,也不持有 `ToastCenter`。
- 字体:全部用系统默认字体(`Font::default()`),**不得**给地址栏等 UI 上 JetBrains Mono。
- 复用统一组件:页签走产品注入的 `tab_core`/`panel_tab`,图标按钮走 `byteui::interaction::icons::icon_button_entry`,不重新手写 `MouseArea`+`on_enter`/`on_exit`。
- **webview 恒在 iced 之上**:库里任何 iced 条(地址栏行、页签栏)的高度仍由 `byteui::theme::geometry::browser_chrome_top_px()` 一个源头给出;`webview_geometry.rs` 与视图读同一个数(Review Focus 3)。
- `pub(crate)` → `pub` 只改**编译器报告 `dozer-app` 需要**的项;不顺手重命名;`Message` 变体名与载荷原样搬,不增不减。
- 注释沿用现有中文、说"为什么"。门禁:`bash scripts/check-i18n.sh`、`python3 scripts/audit/check_panel_boundary.py`、`bash scripts/check-shared-deps.sh`、`bash scripts/check-log-scope.sh`(只有 `window_events.rs` 那处基线告警)。
- **分支/工作区**:新 worktree `../dozer-panel-browser`,分支 `feat/panel-browser-lib`,从当前 `main` 切出。**不碰主工作区**(有用户未提交的 `Cargo.lock` 与 `crates/dozer-app/web/review-trace/`);本地 `[patch]` 造成的 `Cargo.lock` 改动永不提交。不合并 main、不切 tag、不推送、不发布——Task 6 之后停下等用户目测验收。
- 提交信息末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`;commit 前 `git diff --cached --stat` 看 staged 全貌。
- 验收在 `git archive` 导出的**干净快照**里跑,且快照要补 git 历史(`extensions::git_log::tests` 有 3 个测试读仓库历史),目录名不含 "dozer"。

## Review Focus

spec 没细说、但最可能伤到真实使用者的五类问题,每条落到具体任务的测试:

1. **首页全局浏览器 vs 工作区浏览器的作用域**:`home_browser` 用 `project_id = None`(全局 lane),`ws.browser` 用 `Some(pid)`。`update` 里 `AsyncResult` 的 scope 校验、收藏夹"本项目"分组只在 `Some(pid)` 时出现、`BookmarksLoaded(None, _)` 不会被项目实例接受。→ Task 2 `async_result_with_foreign_scope_is_dropped`、`global_instance_never_shows_project_bookmark_status`。
2. **页签关闭/重排后的索引与 hover 状态**:`hover` / `tooltip_starts` 以 tab 序号为键(历史上有"关闭后孤儿 hover 卡在最右页签"的坑,见记忆 tab-close-hover-orphan)。搬迁后关闭中间页签、`reorder_tab` 后,hover 与 tooltip 计时不残留到别的页签;关到最后一个页签时回落到一个 `about:blank`。→ Task 1 `closing_middle_tab_clears_hover_and_tooltip_state`、`closing_last_tab_leaves_one_blank_tab`。
3. **webview 让位高度单一来源**:视图的页签栏+地址栏高度与 `webview_geometry` 读同一个 `browser_chrome_top_px()`;收藏夹侧栏展开时 `bookmarks_open()` 仍驱动 `ShellState::browser_bookmarks_open`。→ Task 4 `chrome_height_has_one_source`(库内断言视图用的高度函数就是 `browser_chrome_top_px`),Task 5 保留 `webview_geometry` 既有的 `browser_bookmarks_open` 用例不改。
4. **地址栏焦点与全选一次性位**:`CaptureAddrFocus`/`addr_field_id`/`take_addr_focused` 搬进库后,`set_addr_focused` 仍是"每帧由 iced 真相写入",`take_addr_select_all_pending` 消费即复位且不会被第二次读取再触发。→ Task 3 `addr_select_all_pending_is_one_shot`。
5. **本地路径/危险 URL 输入**:地址栏提交 `file:///…`、`/Users/x`、空串、纯空白、含空白的 URL、超长 URL 时的行为不变(`error.local_path` 内联提示、不开页签、不崩)。→ Task 3 `addr_submit_rejects_local_paths_and_blank_input`(把现有行为钉成黄金,先于搬迁生成)。

---

## File Structure

| 文件 | 职责 |
|------|------|
| `crates/dozer-panel-browser/Cargo.toml` | 依赖同 `dozer-panel-project`,去掉 `bytegit`/`trash` |
| `src/lib.rs` | `WEB_OWNER = "dozer.web"`、模块声明、重新导出 |
| `src/tabs.rs` | `BrowserTab`、`Tabs`(开/选/关/重排/改标题/改 URL/地址草稿/焦点位) |
| `src/addr_focus.rs` | `addr_field_id`、`take_addr_focused`、`CaptureAddrFocus` |
| `src/bookmarks.rs` | `BookmarkStatus`、`bookmark_status`、`optimistic_add/remove`、`OPTIMISTIC_BOOKMARK_ID` |
| `src/state.rs` | `NavAction`、`Message`、`State`、`TabHover`、`result_project` |
| `src/update.rs` | `update`、`request_bookmarks_refresh` |
| `src/view.rs` | `view`(页签栏、地址栏行、收藏夹侧栏、空态) |
| `src/adapter.rs` | `BrowserDisplay`、`BrowserRegion`、`TextInputTarget`、`BrowserWebviewSpec`、`TAB_TOOLTIP_DELAY` |
| `src/i18n.rs` + `locales/{en,zh-CN}.json` | `t()`;locale 文件从 `dozer-app/locales/panels/web/` `git mv` |
| `src/testing.rs`(`#[cfg(test)]`) | `FakeBookmark`(自 `dozer-app/src/service_fakes.rs` 复制最小版) |
| `crates/dozer-app/src/extensions/browser.rs` | 缩成重新导出 + `BrowserDisplay` 的 Dozer 实现 + `From<BrowserWebviewSpec> for WebviewSpec` |

---

### Task 0: worktree 与基线

**Files:** 无改动(基线记在 `$TMPDIR/browser-lib/`,不提交)。

- [ ] **Step 1: 建 worktree 并核对起点**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-panel-browser -b feat/panel-browser-lib main
git -C ../dozer-panel-browser log --oneline -1 && git -C . status --short | wc -l   # 记下主工作区改动数,全程不得变
```

沙箱不让写兄弟目录时,改用 `/private/tmp` 下独立克隆并在报告开头说明。

- [ ] **Step 2: 记测试基线(在 worktree 内)**

```bash
B="${TMPDIR:-/tmp}/browser-lib"; mkdir -p "$B"
cargo test -p dozer-app -- extensions::browser 2>&1 | tee $B/baseline-browser.log | grep -E "^test result"
cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" | tee $B/baseline-all.log
bash scripts/check-i18n.sh; python3 scripts/audit/check_panel_boundary.py; bash scripts/check-shared-deps.sh; bash scripts/check-log-scope.sh
```

Expected: 门禁全过;`extensions::browser` 的测试条数写进 Task 1 的提交说明。(基线数字开工时自己记,不抄本文。)

---

### Task 1: 新库骨架 + `Tabs` 搬迁

**Files:**
- Create: `crates/dozer-panel-browser/{Cargo.toml,src/lib.rs,src/tabs.rs,src/adapter.rs,src/i18n.rs}`
- Move: `crates/dozer-app/locales/panels/web/{en,zh-CN}.json` → `crates/dozer-panel-browser/locales/`
- Modify: `Cargo.toml`(workspace members)、`crates/dozer-app/Cargo.toml`(加依赖)、`crates/dozer-app/src/extensions/browser.rs`(`Tabs`/`BrowserTab` 改为重新导出)、`scripts/check-i18n.sh` 及其 `scripts/i18n/*.py`(把库内 locales 目录纳入;照 `dozer-panel-todo` 的做法)

**Interfaces:**
- Produces: `dozer_panel_browser::{BrowserTab, Tabs, WEB_OWNER}`;`Tabs` 的公开方法与现 `browser.rs` 中完全相同(`tabs()`、`active_idx()`、`open_url`、`select`、`close`、`reorder`、`apply_title`、`apply_url`、`desired_webviews() -> Vec<BrowserWebviewSpec>`、`active_webview_id()` 等)。
- `adapter::BrowserWebviewSpec { pub id: usize, pub url: String, pub visible: bool }`。
- `i18n::t(key: &str, fallback: &str, locale: &str) -> String`(owner `dozer.web`,实现照 `dozer-panel-todo/src/i18n.rs`)。

- [ ] **Step 1: 先写失败测试(新库 `tabs.rs` 的 `#[cfg(test)]`)**:把 `extensions/browser.rs` 里 `Tabs` 相关的现有测试**原样复制**到库(期望值不改),再追加 Review Focus 2 的两条:
  `closing_middle_tab_clears_hover_and_tooltip_state`(此条属 `State`,若 `State` 尚未搬则写在 Task 2,这里先写 `closing_last_tab_leaves_one_blank_tab`:关掉唯一页签后 `tabs().len()==1` 且 `tabs()[0].url=="about:blank"`)。
  另加 `desired_webviews_maps_one_to_one`:3 个页签、激活第 2 个 → 3 项,`visible` 仅第 2 项为 true,`id` 与页签 id 一致。
- [ ] **Step 2: 运行确认失败**:`cargo test -p dozer-panel-browser` → 编译失败(crate 不存在/类型未定义)。
- [ ] **Step 3: 建 crate**:`Cargo.toml` 抄 `dozer-panel-project/Cargo.toml`(删 `bytegit`、`trash`,加 `tracing = "0.1"`),加入 workspace members;把 `Tabs`/`BrowserTab`/`title_for` 搬入 `tabs.rs`,`desired_webviews` 改返回 `BrowserWebviewSpec`;`i18n.rs` 照 todo;`git mv` locale 文件。
- [ ] **Step 4: Dozer 侧接线**:`dozer-app` 依赖新库;`extensions/browser.rs` 删掉被搬走的定义,改 `pub use dozer_panel_browser::{BrowserTab, Tabs};`;在 `preview/webview.rs` 旁(或 `browser.rs`)加 `impl From<BrowserWebviewSpec> for WebviewSpec`,字段填值与现 `desired_webviews` 里完全相同(`editor_binding/reserve/loading_generation/surface: None`、`park_offscreen: false`、`display_i18n: None`)。`dozer-app` 的 `State::desired_webviews` 暂时仍在 `browser.rs`,用 `.map(Into::into)`。
- [ ] **Step 5: 运行**:`cargo test -p dozer-panel-browser && cargo test -p dozer-app -- extensions::browser` → PASS;`bash scripts/check-i18n.sh` → PASS(`web` 的 locale 现在由库提供,门禁脚本要认到库内路径)。
- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/dozer-panel-browser crates/dozer-app scripts
git commit -m "refactor(browser): 新建 dozer-panel-browser 并迁移 Tabs 与文案"
```

---

### Task 2: 收藏夹逻辑 + `State` / `Message` / `update` 搬迁

**Files:**
- Create: `src/bookmarks.rs`、`src/state.rs`、`src/update.rs`、`src/testing.rs`
- Modify: `src/adapter.rs`(加 `TextInputTarget`)、`crates/dozer-app/src/extensions/browser.rs`、`crates/dozer-app/src/app/update.rs`(`TextInputMenuOpen` 转换)

**Interfaces:**
- Produces(原样搬,仅换路径):`NavAction`、`Message`(变体与载荷一字不改,唯一例外 `TextInputMenuOpen(adapter::TextInputTarget)`)、`State`(含 `with_initial_url`、`bookmarks_open`、`addr_focused`、`set_addr_focused`、`take_addr_select_all_pending`、`active_webview_id`、`desired_webviews`、`advance_hover_anims`、`any_hover_active`、`next_tooltip_wake`、`reorder_tab`、`tab_count`、`active_tab_idx`)、
  `pub fn update(state: &mut State, msg: Message, project_id: Option<i64>, service: &Arc<dyn BookmarkService>, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`、`pub fn request_bookmarks_refresh(..)`(签名照现状)、`Message::result_project(&self) -> Option<i64>`。
- `adapter::TextInputTarget { pub id: iced_widget::core::widget::Id, pub secure: bool }`(同 `dozer-panel-todo`);Dozer 在 `app/update.rs` 的 `Message::Browser(browser::Message::TextInputMenuOpen(target))` 臂里转成 `crate::app::TextInputTarget`。

- [ ] **Step 1: 先写失败测试(库内)**:复制 `browser.rs` 里 `State`/`update`/收藏夹相关现有测试,`FakeBookmark` 在 `testing.rs` 里定义(最小实现 `BookmarkService`,行为与 `service_fakes::FakeBookmark` 一致);测试里的 `panel_host::testing::{TIMEOUT, runtime}` 换成库内等价小函数。追加:
  - `async_result_with_foreign_scope_is_dropped`:`update(.., project_id=Some(1))` 收到 scope 为 `project:2` 的 `AsyncResult(BookmarksLoaded(Some(2), Ok(vec![x])), ..)` → `State` 书签列表不变。
  - `global_instance_never_shows_project_bookmark_status`:`project_id=None` 的实例,即使列表里有 `scope=Project` 的同 URL 项,`bookmark_status(.., None)` 的 `project` 为 `None`。
  - `closing_middle_tab_clears_hover_and_tooltip_state`:3 个页签,对第 2 个 `Hover(1,false,true)` 并记 tooltip 起点;`CloseTab(1)` 后 `hover_progress`/`hover_tooltip_ready` 对新的第 2 个页签均为未悬停。
  - 若该条在现实现下失败(即发现真实 bug),**停下报告**,不要悄悄在搬迁里修;修复单独成提交。
- [ ] **Step 2: 运行确认失败**:`cargo test -p dozer-panel-browser` → FAIL(未定义)。
- [ ] **Step 3: 搬 `bookmarks.rs`/`state.rs`/`update.rs`**:日志 `dozer_core::log_debug!(LOG, ..)` 换 `tracing::debug!`(target 固定 `dozer::panel::web`);`crate::panel_host::{RequestTrackers, scope_of, project_of, RequestTokenWrapExt}` 换 `dozer_panel_host::…`(同名);`crate::app::HOVER_TOOLTIP_DELAY` 换 `adapter::TAB_TOOLTIP_DELAY`(`pub const … = Duration::from_secs(2)`)。
- [ ] **Step 4: Dozer 侧**:`browser.rs` 改 `pub use dozer_panel_browser::{Message, NavAction, State, update, request_bookmarks_refresh, …}`;`app/state.rs` 的 `HOVER_TOOLTIP_DELAY` 改为 `pub(crate) const … = dozer_panel_browser::TAB_TOOLTIP_DELAY;`(消除两份 2 秒常量);`app/update.rs` 加上述 `TextInputTarget` 转换。
- [ ] **Step 5: 运行**:`cargo test -p dozer-panel-browser && cargo test -p dozer-app` → PASS;条数 = Task 0 基线 + 新增。
- [ ] **Step 6: Commit**:`refactor(browser): 迁移 State/Message/update 与收藏夹逻辑到 dozer-panel-browser`

---

### Task 3: 地址栏焦点机制 + 提交校验黄金

**Files:**
- Create: `src/addr_focus.rs`
- Modify: `src/state.rs`、`crates/dozer-app/src/extensions/browser.rs`(重新导出 `addr_field_id`/`take_addr_focused`/`CaptureAddrFocus`)

**Interfaces:**
- Produces:`pub fn addr_field_id() -> iced_widget::core::widget::Id`、`pub fn take_addr_focused() -> bool`、`pub struct CaptureAddrFocus`(`impl Operation<()>`),签名与现状一致;`dozer-app` 里 `window_events.rs`/`app.rs` 的 `browser_addr_focused`/`set_browser_addr_focused`/`take_addr_select_all_pending` 调用点不改。

- [ ] **Step 1: 先写失败测试**(**注意:这两条先在改动前的 `dozer-app` 里对现实现跑绿,把结果当黄金,再搬**):
  - `addr_select_all_pending_is_one_shot`:`set_addr_focused(true)` 触发全选位后,第一次 `take_addr_select_all_pending()` 为 true,第二次为 false。
  - `addr_submit_rejects_local_paths_and_blank_input`:对输入 `""`、`"   "`、`"file:///etc/passwd"`、`"/Users/x/a.html"`、`"https://example.com/"+"a"*10_000` 各 `AddrInput` + `AddrSubmit`;断言:前四条 `tabs().len()` 不变,其中本地路径两条 `error` 为 `error.local_path`、空白两条无新页签;超长 URL 不 panic(期望值以现行为为准,写成字面量)。
- [ ] **Step 2: 运行确认**:库内 FAIL(未定义),`dozer-app` 内现实现 PASS(证明黄金取自旧行为)。
- [ ] **Step 3: 搬 `addr_focus.rs`**;`take_addr_focused` 若用 `static`(进程级),保持单份且在库里(Review Focus:不得出现库与 Dozer 各一份)。
- [ ] **Step 4: 运行** `cargo test -p dozer-panel-browser && cargo test -p dozer-app -- extensions::browser app::` → PASS。
- [ ] **Step 5: Commit**:`refactor(browser): 地址栏焦点机制搬进库并钉住提交校验行为`

---

### Task 4: 视图搬迁(`BrowserDisplay` 适配)

**Files:**
- Create: `src/view.rs`
- Modify: `src/adapter.rs`、`crates/dozer-app/src/extensions/browser.rs`(实现 `BrowserDisplay`、`view` 薄包装)、`crates/dozer-app/src/app/view.rs`(`PanelKind::Web => browser::view(..)` 调用点,参数按需调整)

**Interfaces:**
- Consumes: Task 2 的 `State`/`Message`。
- Produces(`adapter.rs`):

```rust
pub struct BrowserRegion { pub background: Option<Color>, pub padding: Padding, pub gap: f32 }
pub struct TabView { pub title: String, pub active: bool, pub hover_t: f32, pub close_hover_t: f32, pub show_tooltip: bool }
pub trait BrowserDisplay {
    fn region(&self) -> BrowserRegion;                       // = theme::region::browser_pane() 的投影
    fn locale(&self) -> &str;
    fn tab_divider<'a, M: 'a>(&self) -> Element<'a, M, Theme, Renderer>;
    fn tab<'a, M: Clone + 'a>(&self, v: TabView, on_select: M, on_close: M,
        title_hover: impl Fn(bool) -> M + 'a, close_hover: impl Fn(bool) -> M + 'a)
        -> Element<'a, M, Theme, Renderer>;                  // Dozer: chrome::tab_widget::panel_tab
    fn bookmarks_divider<'a, M: Clone + 'a>(&self, on_drag: M) -> Element<'a, M, Theme, Renderer>; // Dozer: app::divider_bar(Divider::BrowserBookmarksSplit, ..)
}
pub fn chrome_top_px() -> f32   // 恒等于 byteui::theme::geometry::browser_chrome_top_px()
pub fn view<'a>(display: &impl BrowserDisplay, state: &'a State, project_id: Option<i64>, bookmarks_split: f32,
    width: Length, outer: Border, mirror: bool) -> Element<'a, Message, Theme, Renderer>
```

- [ ] **Step 1: 先写失败测试**:`chrome_height_has_one_source`(`chrome_top_px() == byteui::theme::geometry::browser_chrome_top_px()`);用一个测试用 `FakeDisplay` 实现 trait,断言 `view(..)` 在 `bookmarks_open=true/false`、`mirror=true/false` 四种组合下都能构建(不 panic),且 `bookmarks_open=false` 时不调用 `bookmarks_divider`(FakeDisplay 内计数器为 0)。再把 `browser.rs` 里现有视图相关测试(若有)原样搬来。
- [ ] **Step 2: 运行确认失败**。
- [ ] **Step 3: 搬 `view`**:`theme::region::browser_pane()` → `display.region()`;`panel_tab(PanelTabArgs{..})` → `display.tab(..)`;`tab_divider()` → `display.tab_divider()`;`crate::app::divider_bar(..)` → `display.bookmarks_divider(Message::ColumnDragStart)`;`workspace::{lh, split_portions}` 若为纯函数且只此处用,搬进库为私有函数,否则经 trait 注入(先 `grep` 其它调用点再定,别重复实现两份)。字体不得上等宽字体。
- [ ] **Step 4: Dozer 侧**:`impl BrowserDisplay for App`(放 `extensions/browser.rs`),`app/view.rs` 调用点改为 `dozer_panel_browser::view(app, &ws.browser, ..)`。
- [ ] **Step 5: 运行** `cargo test -p dozer-panel-browser && cargo test -p dozer-app` → PASS;`cargo clippy -p dozer-panel-browser -p dozer-app --all-targets` 无新增告警。
- [ ] **Step 6: Commit**:`refactor(browser): 视图搬进库,页签/分隔条/区域样式经 BrowserDisplay 注入`

---

### Task 5: Dozer 收口与门禁

**Files:**
- Modify: `crates/dozer-app/src/extensions/browser.rs`(只剩重新导出 + 适配,确认 < 150 行)、`scripts/audit/check_panel_boundary.py`(把 `dozer-panel-browser` 加入与 `dozer-panel-conversations` 同类的"禁止依赖"校验,第 238 行附近)、`scripts/audit/panel-boundary.baseline.json`(`--update`,只许 R-APP/R-WS 数下降)
- Test: 保留 `webview_geometry.rs` 既有 `browser_bookmarks_open` 用例不改。

- [ ] **Step 1: 写失败测试** `scripts/audit/test_unit_edges.py` 里加一条:`dozer-panel-browser` 的 Cargo 依赖里出现 `dozer-app`/`dozer-client`/`dozer-core`/`wry`/`winit` 时门禁报错(用临时 manifest 夹具,模式照 conversations 那条)。
- [ ] **Step 2: 运行确认失败**:`python3 -m unittest scripts/audit/test_unit_edges.py`(命令照该文件现有用法)。
- [ ] **Step 3: 实现门禁扩展**;`python3 scripts/audit/check_panel_boundary.py --update`,`git diff scripts/audit/panel-boundary.baseline.json` 确认只有下降。
- [ ] **Step 4: 清理**:`grep -rn "crate::extensions::browser::" crates/dozer-app/src` 的调用点里,凡只用重新导出的保持不动;确认 `crates/dozer-app/locales/panels/web` 已不存在(Review Focus 的"唯一来源"同理)。
- [ ] **Step 5: 运行** `bash scripts/check-i18n.sh && python3 scripts/audit/check_panel_boundary.py && bash scripts/check-shared-deps.sh && bash scripts/check-log-scope.sh` → 全过(log-scope 只有基线那一处)。
- [ ] **Step 6: Commit**:`chore(browser): 收口 extensions/browser.rs 并扩展面板边界门禁`

---

### Task 6: 干净快照全量验证与报告

**Files:** Create: `docs/superpowers/reviews/2026-10-27-panel-browser-lib-report.md`

- [ ] **Step 1: 导出快照并补历史**:`git archive HEAD` 解到 `/private/tmp/bb-lib-snap`(目录名不含 "dozer"),`git init` + `git fetch <worktree> HEAD` + `git reset --soft FETCH_HEAD`;用独立 `CARGO_TARGET_DIR`。
- [ ] **Step 2: 跑全套**:`cargo build --workspace && cargo test --workspace --no-fail-fast && cargo clippy --workspace --all-targets && cargo fmt --check`,再 Task 5 Step 5 的四个门禁,再 `cargo tree -p dozer-panel-browser | grep -E "dozer-(app|client|core)|dozerd|wry|winit"`(Expected: 无输出)。
- [ ] **Step 3: 核对测试数**:总数 = Task 0 基线 + Task 1–4 新增;`extensions::browser` 搬走的条数在库里逐条能找到(`cargo test -p dozer-panel-browser -- --list` 对照基线日志)。偶发失败单独重跑量化,写次数与失败数,不靠重跑到绿。
- [ ] **Step 4: 写报告**:基线/终态数字、偏差(哪些测试是新增)、Review Focus 五条各对应的测试名、任何"发现的真实 bug"。
- [ ] **Step 5: 请用户目测验收后停下**:验收清单——首页右栏全局浏览器(默认 `https://byteboy.ai`)、工作区 Web 面板开/关/拖拽页签、地址栏输入/回车/右键菜单(菜单打开时 webview 隐藏)、后退/前进/刷新、页内链接跳转后地址栏跟随、`window.open` 新开页签、收藏(全局/本项目)与侧栏、收藏夹分割线拖动、面板移到右侧(mirror)、中文输入法在地址栏输入。**合并、切 tag、推送、Digger 重钉都不在本 plan 内。**
- [ ] **Step 6: Commit**:`docs(browser): 浏览器面板抽库验证报告`

---

## 不在本 plan 内

- Digger 接入浏览器面板(需要 `native_host` 承载 wry webview,走 S7/S8 同类交接提示,另起一期)。
- `wry` 池、`IpcMessage::Browser*`、`NavAction` 的执行、`TabGroup::Browser` 拖拽、`Divider::BrowserBookmarksSplit` 几何、`browser_desired` 的浮层隐藏逻辑:留在 `dozer-app`(它们依赖 `App`/winit/wry)。
- CDP agent 浏览器(已推迟,见记忆);收藏夹功能改动;任何 UI 变化。
