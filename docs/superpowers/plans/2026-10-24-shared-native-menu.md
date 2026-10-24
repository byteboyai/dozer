# 共享原生菜单(NSMenu)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Dozer 的 macOS 原生菜单(`chrome/native_menu.rs`,NSMenu)抽成 byteui 的共享模块,让 Digger 的预览页签右键菜单、预览页签溢出下拉、文件树右键菜单都用原生菜单——菜单弹出时**不再把文件内容(原生 webview)藏起来**。

**Architecture:** 原生菜单是 AppKit 的真窗口层级,天然浮在 wry webview 之上,不需要 iced 浮层加"弹出时隐藏 webview"。该模块与 Dozer 的耦合只有两处(一条日志宏、`install_content_view` 收 `winit` 窗口),其余已是 byteui 类型(`IconKind`、主题色),所以按 `byteui` → Dozer 改引用 → Digger 接入三步走。Dozer 侧用薄 shim `pub use byteui::native_menu::*;` 保持 92 处调用点不动。

**Tech Stack:** Rust、iced 0.14、`objc2` 0.6 / `objc2-app-kit` 0.3 / `objc2-foundation` 0.3、`resvg` / `usvg` / `tiny-skia`(与 iced_wgpu 同一精确版本)、`raw-window-handle` 0.6。

**Spec:** `crates/dozer-app/src/chrome/native_menu.rs` 文件头引用的 `docs/superpowers/specs/2026-09-16-native-context-menu-design.md`(原设计);本计划只搬运与接入,不改菜单外观与行为。

## Global Constraints

- **搬运,不改行为:** `native_menu.rs` 的函数体、尺寸常量(`SEP_ROW_HEIGHT` 等)、`dark_equivalent` 配色翻译、`MenuItemView` 的 `define_class!` 一律原样搬。Dozer 菜单外观字节不变。
- **仅 macOS:** 整个模块 `#![cfg(target_os = "macos")]`;依赖放 `[target.'cfg(target_os = "macos")'.dependencies]`;非 macOS 构建不受影响。
- **依赖版本:** `resvg = "=0.45.1"`、`usvg = "=0.45.1"`、`tiny-skia = { version = "=0.11.4", features = ["png"] }` 必须与 `iced_wgpu` 用的一致(Dozer `Cargo.toml` 125–132 行的注释已说明:避免重复编译);`objc2*` 版本与 Dozer、Digger 现用的一致,锁文件里不得出现两份。
- **iced 版本:** byteui 与两个产品的 iced 必须同为 0.14,升 byteui 时 Dozer 与 Digger 同日一起升(项目既有约定)。
- **库边界:** byteui 是独立仓库(`/Users/chrischiang/Projects/CoralProjects/byteboy/byteui`,`byteboyai/byteui`),只被 git tag 引用;不引入 `dozer_*` / `bytehost_*` 依赖;日志用 `tracing`(byteui 属于"独立库用裸 tracing"一类,`check-log-scope.sh` 不覆盖它)。
- **`install_content_view` 签名:** 接收 `&dyn raw_window_handle::HasWindowHandle`,不依赖 `winit`(Digger 的窗口来自 iced 的 `window::run`,Dozer 传 `&winit::window::Window`,二者都实现该 trait)。
- **对外动作先问用户:** 给 byteui 打 tag 与推送、给 Dozer 打 `libs-*` tag 与推送、合并 Dozer `main`(`main` 上有自动发版流程)。
- **隔离环境:** 真机验证用隔离的数据目录与临时 `HOME`,不碰真实数据与常驻守护进程。

## Review Focus

1. **窗口内容 view 还没登记就弹菜单**(`install_content_view` 尚未被调用):期望 `show` 返回 `None` 并记一条警告,不 panic。→ Task 1。
2. **菜单项为空:** 期望不弹、返回 `None`。→ Task 1。
3. **macOS 上同一次右键不能既弹原生菜单又出 iced 浮层:** Digger 现在对 `ShowContextMenu` / `ShowNativeMenu` 两种请求都建 iced 浮层状态,接入后 macOS 必须跳过。→ Task 6。
4. **菜单弹出时不再隐藏文件内容:** `preview_overlay_open()` 在"只有原生菜单"时必须为假;iced 浮层(删除确认、移动卡片)仍要隐藏。→ Task 7。
5. **菜单锚点贴近窗口右 / 下边缘:** 期望钳进窗口内(沿用原有钳位公式),不跑出窗体。→ Task 1(原有逻辑,测试钉住)。

---

## Part A — byteui(新分支 `feat/native-menu`,release 前先问用户)

### Task 1: 把 `native_menu` 搬进 byteui

**Files:**
- Create: `byteui/src/native_menu.rs`(自 `dozer/crates/dozer-app/src/chrome/native_menu.rs` 搬入)
- Modify: `byteui/src/lib.rs`(`#[cfg(target_os = "macos")] pub mod native_menu;`)、`byteui/Cargo.toml`(target 依赖 + `tracing`)、`byteui/CHANGELOG.md`、版本 `0.4.5` → `0.4.6`

**Interfaces:**
- Produces(与 Dozer 现状同名同形,不改):
  - `pub enum Item<Msg> { Entry { icon: Option<IconKind>, icon_color: Option<Color>, label: String, color: Color, enabled: bool, msg: Msg }, Separator }`,`Item::entry(icon, label, msg)`。
  - `pub fn show<Msg: Clone>(items: Vec<Item<Msg>>, view_pos: (f32, f32)) -> Option<Msg>`、`pub fn show_align_no_icon_left<Msg: Clone>(items, view_pos) -> Option<Msg>`。
  - `pub fn install_content_view(window: &dyn raw_window_handle::HasWindowHandle)`(**签名变化**:原为 `&winit::window::Window`)。
- 日志:原 `dozer_core::scope!(LOG, ...)` / `dozer_core::log_warn!(LOG, ...)` 换成 `tracing::warn!(target: "byteui::native_menu", ...)`。

- [ ] **Step 1: 搬文件与测试。** 把 `native_menu.rs` 整份(含 `mod tests` 与 `menu_item_view`)复制进 byteui;把文件内对 `crate::chrome::menu.rs` 的注释引用改成"对齐 Dozer 的 iced 版菜单项尺寸口径"。**先只搬、不改逻辑,确认测试在 byteui 里原样通过。**
- [ ] **Step 2: 改两处耦合并补依赖。** 日志宏换 `tracing`;`install_content_view` 改接 `&dyn HasWindowHandle`;`Cargo.toml` 加 `[target.'cfg(target_os = "macos")'.dependencies]`(`objc2`、`objc2-app-kit`、`objc2-foundation`、`raw-window-handle`、`resvg`、`usvg`、`tiny-skia`,版本见 Global Constraints),并加 `tracing = "0.1"`。
- [ ] **Step 3: 补测试(先失败):**
  - `show_without_a_registered_content_view_returns_none`(**Review Focus 1**:未调用 `install_content_view` 时 `show(vec![Item::entry(None, "x", 1)], (0.0, 0.0))` 返回 `None`,不 panic);
  - `show_with_no_items_returns_none`(**Review Focus 2**);
  - 原有的钳位、图标栅格化、`dark_equivalent` 测试照搬(**Review Focus 5**)。
  这两条测试要在不弹真实菜单的前提下运行:`show` 在内容 view 缺失与空表两种情况下都先于任何 AppKit 调用返回(原代码已如此,只补测试)。
- [ ] **Step 4: 验证。** `cargo test`、`cargo clippy --all-targets`、`cargo fmt --check` 在 byteui 仓库全过;`cargo build --target aarch64-apple-darwin`;`cargo tree -d` 里 `objc2*`、`resvg`、`usvg`、`tiny-skia` 各只有一份(重复说明版本没对齐,停下来报告)。
- [ ] **Step 5: 提交** `feat(native_menu): macOS NSMenu moved from dozer-app (0.4.6)`。**打 tag `v0.4.6` 与推送先问用户。**

---

## Part B — Dozer(分支 `feat/shared-native-menu`,worktree `../dozer-native-menu`)

> **基线不是 `main`,是 `preview/dozer-fixes`(`70ec02af`):** 它是 `main` 加上两个应用修复(页签气泡结构、Agent 溢出下拉)和整条库修复链(预览桥接抽库、`attach` 孤儿会话、制表符、Nerd Font 图标)合并后的结果,这些库修复已随 `libs-2026-10-24c` 被 Digger 使用。**如果新 `libs-*` tag 从 `main` 切,Digger 重钉后会丢掉这些库修复。** 开工先核对 `git rev-parse --short preview/dozer-fixes` 是 `70ec02af`,不是就停下来问。

### Task 2: Dozer 改引用 byteui 的 `native_menu`

**Files:**
- Modify: `Cargo.toml` / `crates/*/Cargo.toml`(byteui 的 tag `v0.4.5` → `v0.4.6`,全部一起改)、`crates/dozer-app/src/chrome/native_menu.rs`(整文件换成 shim)、`Cargo.lock`
- Test: `crates/dozer-app/src/chrome/native_menu.rs` 里原有的 `mod tests` 随模块搬走(已在 Task 1 到 byteui)

**Interfaces:**
- Consumes: Task 1 的 `byteui::native_menu::{Item, show, show_align_no_icon_left, install_content_view}`。
- Produces: `crate::chrome::native_menu` 保持原路径可用:`pub use byteui::native_menu::*;`,**92 处调用点与 `Item` 构造一律不改**。

- [ ] **Step 1: 记基线。** 改动前 `cargo test --workspace --no-fail-fast`,记下总数 N 与 `native_menu` 测试条数 K(`-- --list | grep native_menu`)。
- [ ] **Step 2: 换 shim 并升 tag。** `native_menu.rs` 只留 `#![cfg(target_os = "macos")]`、一行文档与 `pub use byteui::native_menu::*;`;byteui 的 tag 一起改到 `v0.4.6`。`install_content_view(&window)` 的调用点(`todo_detail_overlay.rs` 等 6 处)因 `&winit::window::Window` 可协变成 `&dyn HasWindowHandle` 而不用改;若编译报类型不符再按报错处理。
- [ ] **Step 3: 清理不再需要的直接依赖。** 若 `objc2-app-kit`、`resvg`、`usvg`、`tiny-skia` 在 `dozer-app` 里除 `native_menu.rs` 外已无其它使用(`grep`),从 `dozer-app/Cargo.toml` 删掉并在报告里写明;仍有使用的保留。
- [ ] **Step 4: 验证行为不变。** `cargo test --workspace`:总数 = N − K(K 条测试搬去了 byteui);`cargo clippy --workspace --all-targets` 与基线相同;`scripts/check-shared-deps.sh`、`cargo metadata --locked`、`cargo tree -d`(`objc2*` 等不得出现两份);`bash scripts/check-log-scope.sh`(基线既有的 `preview_panel.rs:233` 不算本次引入)。
- [ ] **Step 5: 提交** `refactor(app): native_menu 改引用 byteui v0.4.6`。

### Task 3: 新 `libs-*` tag(**对外动作,先问用户**)

- [ ] **Step 1:** 干净快照里全量验证(路径不含 "digger";`git archive` 无 `.git` 时 `git_log` 的 3 条测试失败是已知环境失败)。
- [ ] **Step 2:** **停下来问用户**:是否合并 Dozer `main`(有自动发版流程)、tag 名(建议 `libs-2026-10-24d` 之后的下一个字母)、是否推送。Digger 按 tag 引用,**只打 tag 不合 `main` 也能用**(此前两次都是这样做的)。
- [ ] **Step 3:** 记下 tag 内钉的 `bytehost` / `byteui` / `bytegit` 版本,供 Task 4 核对(byteui 应为 `v0.4.6`)。

---

## Part C — Digger(分支 `feat/native-menu`,从 Digger `main` 切出,worktree `../digger-native-menu`)

### Task 4: 依赖升到新 tag,并登记窗口内容 view

**Files:**
- Modify: `Cargo.toml`(`byteui` → `v0.4.6`;所有 `dozer-*` → Task 3 的新 tag)、`Cargo.lock`、`crates/desktop/src/product.rs`(`Msg::WindowReady` 处理里登记内容 view)
- Test: `crates/desktop/src/product.rs`

**Interfaces:** Consumes `byteui::native_menu::install_content_view(&dyn HasWindowHandle)`。

- [ ] **Step 1: 升依赖并核对。** 改 `byteui` 与全部 `dozer-*` 的 tag;`cargo update --workspace`;`cargo metadata --locked`、`cargo tree -d`、`scripts/check-shared-deps.sh`、`scripts/check-host-deps.sh` 全过;`bytehost` / `byteui` / `bytegit` 版本与 tag 内钉的逐个一致。
- [ ] **Step 2: 登记内容 view。** 在主窗口就绪时(`Msg::WindowReady(id, _)`)用 `iced::window::run(id, |w| byteui::native_menu::install_content_view(w))` 登记一次(`#[cfg(target_os = "macos")]`)。测试:`window_ready_registers_the_content_view_on_macos`——用现有产品壳测试夹具走 `WindowReady`,断言返回的 `Task` 非空(不执行真实 AppKit 调用)。
- [ ] **Step 3: 全量与提交。** `cargo test --workspace` 总数不降;`chore(deps): align digger with dozer libs <tag> and byteui v0.4.6`。

### Task 5: 文件树 / 页签右键菜单改用原生菜单

**Files:**
- Create: `crates/desktop/src/adapters/native_menu.rs`(库 `MenuItem` → `byteui::native_menu::Item` 的纯函数)
- Modify: `crates/desktop/src/adapters/files.rs`(`take_requests`:macOS 不再为 `ShowContextMenu` / `ShowNativeMenu` 建 iced 浮层状态,改为把 `(items, at)` 留给产品)、`crates/desktop/src/product.rs`(`on_files_message`)、`adapters/mod.rs`
- Test: `adapters/native_menu.rs`、`adapters/files.rs`

**Interfaces:**
- Produces: `pub fn to_native_items<Msg>(items: Vec<dozer_panel_files::menu::MenuItem<Msg>>) -> Vec<byteui::native_menu::Item<Msg>>`(`Entry` → `Item::Entry { icon, icon_color: None, label, color: byteui::theme::color::current().body, enabled, msg }`,`Separator` → `Item::Separator`,保持顺序;与 Dozer `app/update.rs` 的 `run_files_requests` 里的转换一致)。
- `FilesPanel::take_menu_requests(&mut self) -> Vec<(Vec<MenuItem<FilesMessage>>, (f32, f32))>`:取走库排出的"弹菜单"请求(macOS 路径)。

- [ ] **Step 1: 写失败测试:**
  - `to_native_items_preserves_order_and_fields`(条目顺序、`enabled`、`msg` 原样,分隔线位置不变);
  - `on_macos_a_context_menu_request_is_handed_to_the_product_not_drawn_as_an_overlay`(**Review Focus 3**:库发 `ShowContextMenu` 后 `FilesPanel::menu()` 为 `None`,`take_menu_requests()` 有一条);非 macOS 行为不变。
- [ ] **Step 2: 实现转换函数与 `take_menu_requests`**(`take_requests` 里 macOS 分支改为入队而不是建 `MenuOverlay`)。
- [ ] **Step 3: `on_files_message` 接线(`#[cfg(target_os = "macos")]`)。** 处理完库消息后排空 `take_menu_requests()`,对每条用 `byteui::native_menu::show_align_no_icon_left(to_native_items(items), at)`,有选中项就把该消息当作新的 `Msg::Files(msg)` 回投(与 Dozer 的 `proxy.send_event(Message::Files(msg))` 一致)。页签右键(`TabContextMenuOpen`)与文件树右键走同一条路径。
- [ ] **Step 4: 验证。** `cargo test -p digger-desktop` 全过。**真机:** 右键文件树条目、右键预览页签,菜单是原生样式、点选项生效、Esc 关闭。
- [ ] **Step 5: 提交** `feat(files): 文件树与预览页签右键菜单改用原生 NSMenu`。

### Task 6: 预览页签溢出下拉改用原生菜单

**Files:**
- Modify: `crates/desktop/src/product.rs`(拦截预览库的 `TabOverflowToggle`)、`adapters/native_menu.rs`(加 `tab_overflow_items`)
- Test: `adapters/native_menu.rs`、`product.rs`

**Interfaces:** Produces `pub fn tab_overflow_items<Msg: Clone>(entries: &[(usize, String, bool)], on_select: impl Fn(usize) -> Msg) -> Vec<byteui::native_menu::Item<Msg>>`——**与 Dozer `tab_widget::tab_overflow_items` 同形同行为**(只有选中行带 `>` 图标,色取 `cream` / `body`);Dozer 那份在 `chrome/tab_widget.rs` 约 438 行,Digger 写一份等价的(它依赖 Dozer 应用层的 `tab_widget`,不能直接引用)。

- [ ] **Step 1: 失败测试:** `tab_overflow_items_marks_only_the_active_row`、`tab_overflow_items_dispatches_the_row_index`。
- [ ] **Step 2: 实现并拦截。** macOS 上收到 `PM::TabOverflowToggle { .. }` 时,从当前面板的页签列表取 `(下标, 标题, 是否选中)`,弹原生菜单,选中后回投 `Message::SelectTab(i)`;不再让库置 `overflow_anchor`(否则会同时出 iced 下拉)。
- [ ] **Step 3: 验证与提交** `feat(preview): 页签溢出下拉改用原生菜单`;真机:开很多页签,点 `≫` 弹原生菜单,选中项切换页签。

### Task 7: 原生菜单弹出时不再隐藏文件内容

**Files:** Modify `crates/desktop/src/product.rs`(`preview_overlay_open`、`files_menu_is_rendered`)。Test: `product.rs`。

- [ ] **Step 1: 失败测试(**Review Focus 4**):**
  - `a_native_menu_does_not_hide_the_preview`(macOS:`preview_overlay_open()` 在"只有 Files 右键菜单 / 页签溢出"时为假——这两类现在由原生菜单承担,不占 iced 浮层);
  - `an_iced_modal_still_hides_the_preview`(删除确认卡、移动卡片等 iced 浮层存在时仍为真)。
- [ ] **Step 2: 实现。** `preview_overlay_open` 里把"`files.menu().is_some()`"与"页签 `overflow_anchor`"这两项用 `#[cfg(not(target_os = "macos"))]` 包住(非 macOS 仍走 iced 浮层并隐藏页面);`files_menu_is_rendered` 同理。
- [ ] **Step 3: 全量门禁与真机。** 干净快照(路径不含 "digger")里 `cargo metadata --locked --offline`、`cargo test --workspace --no-fail-fast`、`cargo clippy --workspace --all-targets`、`check-host-deps.sh`、`check-shared-deps.sh`,跑完删临时目录。**真机:** 右键预览页签、点 `≫`、右键文件树条目,三种菜单弹出时 `notes.txt` 的内容都保持可见。
- [ ] **Step 4: 报告** `docs/superpowers/reviews/2026-10-<日期>-native-menu-report.md`:每条"通过"附命令与退出码;测试总数与名字守恒;真机项逐条记录。**合并 `main` 先问用户。**

---

## Self-Review

- **目标覆盖:** "菜单弹出时内容不隐藏"→ Task 7;"文件树/页签右键、页签溢出都走原生"→ Task 5、6;"抽成共享模块"→ Task 1–3。
- **类型一致:** `Item`、`show`、`show_align_no_icon_left`、`install_content_view` 在 Task 1 定义,Task 2 以 shim 复用、Task 4–6 引用;`to_native_items`、`take_menu_requests`、`tab_overflow_items` 各在一处定义。
- **风险点:** ① `show` 是同步阻塞的模态追踪,会阻塞 iced 事件循环直到用户选择(Dozer 同样如此,可接受);② Digger 的 `last_cursor` 是否就是内容 view 的逻辑坐标,真机项要确认菜单出现在点击处;③ 若 Digger 的窗口 `HasWindowHandle` 取到的不是内容 view,菜单位置会偏,Task 4 的登记要在真机上确认;④ byteui 与 Dozer 的 tag、推送、合并都是对外动作,逐次问用户;⑤ Dozer 不该在本计划里顺手改别的(包括它"重新加载把下标当 id"的潜在缺陷,单独处理)。
