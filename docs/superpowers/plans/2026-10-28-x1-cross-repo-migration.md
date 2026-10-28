# X1 跨仓库迁移 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Digger 里"临时复制"的页签部件、菜单弹层和可信原生子 webview 执行器,上移到 byteui / bytehost,Dozer 与 Digger 共用同一份,删除两处重复。

**Architecture:** 两个互相独立的子工程,**分两个阶段、各自发版**:阶段 A 把 Dozer `chrome/menu.rs`、`chrome/tab_widget.rs`(原件)抽进 byteui,Dozer 与 Digger 都改为消费;阶段 B 把 Digger `adapters/native_host.rs`(可信原生子 webview 执行器)抽进 `bytehost-desktop`,Digger 的四个原生面板与 markdown 宿主改为消费。Dozer 的外观与行为在两个阶段里都**不许变**。

**Tech Stack:** Rust workspace;iced 0.14(版本必须与 byteui 一致);wry 0.55.1;byteui、bytehost、bytegit 一律按 tag 引用;Dozer 库 crate 按 `libs-YYYY-MM-DD` tag 给 Digger 消费。

**Spec:** 无独立设计稿。依据:Digger `docs/adr/ADR-015-product-owned-daemon.md`(第 111 行"页签部件与菜单弹层…后续应迁入 byteui")、Digger `docs/superpowers/notes/2026-10-18-agent-parity.md`(技术债)、`docs/superpowers/specs/2026-10-01-byteui-standalone-repo-design.md`、`bytehost` 仓库 `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`(§6.4:wry builder 只在 desktop 层有一份)、本目录 `2026-10-27-s7-s8-closeout.md` §5。

## Global Constraints

- 三个共享依赖在两边**逐个一致**:byteui(现 `v0.4.6`)、bytehost(现 `v0.4.12`)、bytegit;`scripts/check-shared-deps.sh` 必须 0 阻断。每次共享库发版 = Dozer 重钉 + **新 Dozer libs tag** + Digger 同日重钉(Dozer 的 `dozer-panel-*` 依赖 byteui/bytehost,Digger 经它们间接消费)。
- 只用 tag 引用,本地联调的 `[patch]` 写在不提交的 `.cargo/config.toml`;不提交由 `[patch]` 造成的 `Cargo.lock` 改动。
- iced 版本与 byteui 一致,升级时一起升;本计划**不升 iced**。
- wire 形状"只追加不修改";本计划不碰 `AppRequest`/`AppReply`。
- Dozer 的可见外观与行为不变:移动是搬代码不是重写,Dozer 现有测试**一条不删不改**(只允许改 `use` 路径)。
- byteui 不得依赖任何业务 crate;bytehost-desktop 允许 iced/wry,不含产品业务 crate;bytehost 用裸 `tracing`,byteui 不引入日志。
- `bytehost-desktop` 里 wry 的每项安全设置(下载、新窗口、开发者工具、媒体权限、导航白名单)必须与迁移前**逐项相同**,且有读生产源码的钉死测试(迁移时随代码一起搬,且仍读到正确文件)。
- 非 macOS 路径"只要求能编译"(该回退"从未实际编译过"),不要求功能。
- 每个共享库发版、推送、打 tag 之前**必须得到用户确认**;每个 worktree/分支独立,不碰 Dozer 主工作区用户未提交的 `Cargo.lock` 与 `crates/dozer-app/web/review-trace/`,不碰 Digger 里 13 份已删除的 S6 文档。
- 门禁一律在干净快照(`git archive`,路径不含 "digger")里跑;偶发测试用重复隔离运行量化,不"重跑到绿"。提交带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`,提交前 `git diff --cached --stat`。

## Review Focus

1. **搬家后 Dozer 像素/度量漂移**:同一页签、同一菜单在缩放 1 和 2 下的尺寸、内边距、圆角必须与搬家前相同(Task 3、Task 4 的黄金度量测试)。
2. **菜单在窗口边缘**:右/下放不下要翻转,窗口比菜单还小时位置与尺寸不得为负或 NaN(Task 3 的 `menu_anchor_*` 测试)。
3. **溢出菜单的边界输入**:0 个、1 个、几百个条目;超长标题要裁切不换行(Task 4 的 `overflow_menu_*` 测试)。
4. **wry 安全设置被搬丢或被"默认值"替换**:wry 默认值不是安全默认,迁移后的 builder 必须仍逐项显式关闭(Task 7 的源码钉死测试,迁移后仍读得到文件)。
5. **共享库钉版漂移**:只升一边会让 `check-shared-deps.sh` 拦截或让 Cargo.lock 里出现两份 byteui/iced(每个发版 Task 末尾的重复依赖检查)。

---

## 文件结构

| 位置 | 动作 | 职责 |
|------|------|------|
| `byteui/src/interaction/menu.rs`(新) | 来自 Dozer `chrome/menu.rs` + Digger `menu_overlay.rs` 的并集 | 菜单项/分隔线/外壳/锚点钳制,界面框架内无业务 |
| `byteui/src/interaction/panel_tabs.rs`(新) | 来自 Dozer `chrome/tab_widget.rs` 的通用部分 | 面板页签、溢出按钮、溢出菜单项与菜单 |
| `byteui/src/feedback/`(扩) | 来自 Dozer `app/view.rs` | `controlled_tooltip`、`top_bar_font` 两个小函数 |
| `dozer/crates/dozer-app/src/chrome/{menu,tab_widget}.rs` | 缩成重导出 + Dozer 专属部件 | 调用点不动 |
| `digger/crates/desktop/src/adapters/{menu_overlay,agent_tabs}.rs` | 删 / 缩成"库显示参数 → byteui 部件"的映射 | |
| `bytehost/crates/bytehost-desktop/src/native_child.rs`(新) | 来自 Digger `adapters/native_host.rs` | 可信原生子 webview 的 Desired/plan/Action/Host/Probe |
| `digger/crates/desktop/src/adapters/{native_host,usage_native,todo_native,group_chat_native,review_native}.rs`、`markdown/host.rs` | 改为消费 | |

---

# 阶段 A:byteui(页签部件 + 菜单弹层)

### Task 1: 盘点三份实现的差异(无代码改动,出报告)

**Files:**
- Create: `dozer/docs/superpowers/specs/2026-10-28-x1-inventory.md`

**Interfaces:**
- Produces: 一张表,逐个符号列出「Dozer 原件 / Digger 复制 / 差异 / 迁入 byteui 后的归宿(通用→byteui,Dozer 专属→留 Dozer,Digger 专属→留 Digger)」。Task 2–4 的范围以它为准。

- [ ] **Step 1:** 对比 Dozer `chrome/menu.rs`(260 行)与 Digger `adapters/menu_overlay.rs`(422 行):逐个公开函数/类型比对,重点记录 Digger 多出的部分(`MenuAnchor` 及其 `clamped`、浮层定位)是否 Dozer 也需要。
- [ ] **Step 2:** 对比 Dozer `chrome/tab_widget.rs`(717 行)与 Digger `adapters/agent_tabs.rs`(618 行):Dozer 的 `tab_render_mode_button`/`tab_tabular_mode_button`/`tab_json_mode_button` 是预览专属,标"留 Dozer";`panel_tab`、`tab_label`、`tab_container_style`、`tab_overflow_button`、`tab_overflow_items`、`tab_overflow_menu` 标"通用"。记录对 `crate::app::{controlled_tooltip, top_bar_font}` 与 `crate::theme` 的依赖(迁移时要么随迁要么参数化)。
- [ ] **Step 3:** 记录 Digger 已知差异(`agent_tabs.rs` 头注:悬停是阶跃不是渐变、提示精度 1 秒)——迁入后是否收敛成一份,写进表里,**由用户裁决**,不自行取舍。
- [ ] **Step 4:** 前置条件核对并写进报告:bytehost 当前是否处于稳定窗口(`git log` 近期无破坏性变更);byteui 现有的 `native_menu` 重导出在非 mac 上是否能编译(`cargo check --target` 无法交叉时,记为"未验证")。
- [ ] **Step 5:** 提交报告。`git add docs/superpowers/specs/2026-10-28-x1-inventory.md && git commit`。报告交用户确认范围后再进 Task 2。

### Task 2: byteui 里先落通用小件

**Files:**
- Modify: `byteui/src/feedback/mod.rs`(新增子模块)、`byteui/src/lib.rs`
- Test: byteui 同文件 `#[cfg(test)]`

**Interfaces:**
- Produces: `byteui::feedback::tooltip::controlled_tooltip<'a, M, R>(content: Element<'a, M, Theme, R>, label: String, position: tooltip::Position, show: bool) -> Element<'a, M, Theme, R>`(签名与 Dozer `app/view.rs:1826` 逐字一致,`M: Clone + 'a`,`R: text::Renderer + 'a`);`byteui::theme::font::top_bar_font() -> Font`(`Weight::Medium` 的 `Font::default()`)。

- [ ] **Step 1:** 在 byteui 写失败测试 `top_bar_font_is_medium_weight`(断言 `weight == Weight::Medium`)与 `controlled_tooltip_ignores_show_like_dozer`(行为照 Dozer 现状:`_show` 不影响结构,测试断言可构造且不 panic)。
- [ ] **Step 2:** 运行 `cargo test -p byteui` 确认因函数不存在而失败。
- [ ] **Step 3:** 实现上述两个函数,**函数体从 Dozer 原样搬**,不改样式常量。
- [ ] **Step 4:** `cargo test -p byteui` 通过;`cargo clippy -p byteui --all-targets`、`cargo fmt --check` 无新增。
- [ ] **Step 5:** 提交(byteui 仓库独立 worktree/分支)。**先不发版**,与 Task 3、4 合并发一个 byteui 版本。

### Task 3: 菜单弹层迁入 `byteui::interaction::menu`

**Files:**
- Create: `byteui/src/interaction/menu.rs`;Modify: `byteui/src/interaction/mod.rs`
- Modify(Dozer): `crates/dozer-app/src/chrome/menu.rs` 缩成 `pub use byteui::interaction::menu::*;`
- Test: byteui 同文件

**Interfaces:**
- Consumes: Task 1 的归宿表(Digger 多出的 `MenuAnchor` 是否并入)。
- Produces: 与 Dozer `chrome/menu.rs` 同名同签名的 `item`、`item_locked`、`item_row`、`item_row_fill`、`icon_leading`、`separator`、`shell`、`shell_frosted`;并入 `pub struct MenuAnchor { pub x: f32, pub y: f32 }` 与 `impl MenuAnchor { pub fn clamped(self, size: (f32, f32), window: (f32, f32)) -> Self }`(行为照 Digger `menu_overlay.rs` 现状:右/下放不下向左/上翻,结果不小于 0)。

- [ ] **Step 1:** 在 byteui 写失败测试:`menu_anchor_clamps_into_the_window`(锚点 `(900, 700)`、菜单 `(220, 300)`、窗口 `(1000, 800)` → `(780, 500)`);`menu_anchor_never_goes_negative_when_the_window_is_smaller_than_the_menu`(窗口 `(100, 100)`、菜单 `(220, 300)` → `(0, 0)`,且各分量 `is_finite()`);`menu_shell_metrics_match_dozer_at_scale_1_and_2`(用 Dozer 搬家前的数值做黄金值:外框圆角 9、内边距 8、项间距 1,缩放 2 时翻倍)。
- [ ] **Step 2:** 运行 `cargo test -p byteui menu` 确认失败。
- [ ] **Step 3:** 把 Dozer `chrome/menu.rs` 的实现搬进 `byteui/src/interaction/menu.rs`,`crate::theme` 改为 `byteui::theme`;并入 `MenuAnchor`。**Dozer 现有的菜单测试随代码搬进 byteui,名字不变。**
- [ ] **Step 4:** byteui 全量测试通过。
- [ ] **Step 5:** 提交(与 Task 2 同分支)。

### Task 4: 页签部件迁入 `byteui::interaction::panel_tabs`

**Files:**
- Create: `byteui/src/interaction/panel_tabs.rs`;Modify: `byteui/src/interaction/mod.rs`
- Modify(Dozer): `crates/dozer-app/src/chrome/tab_widget.rs`:通用函数改为 `pub use`,三个预览模式按钮留在原处
- Test: byteui 同文件

**Interfaces:**
- Consumes: Task 2 的 `controlled_tooltip`/`top_bar_font`;Task 3 的 `menu::{shell, item_row, separator}`。
- Produces: 与 Dozer 现状同名同签名的 `tab_label`、`tab_container_style`、`panel_tab`、`tab_overflow_button`、`tab_overflow_items`、`tab_overflow_menu`(范围以 Task 1 为准)。

- [ ] **Step 1:** 在 byteui 写失败测试:`panel_tab_metrics_match_dozer_at_scale_1_and_2`(内边距 左 10 / 右 4 / 上下 0,选中态 1px 边框 + 圆角 6,关闭按钮边长取 `geometry::tab_button_size()`);`overflow_menu_with_zero_one_and_many_entries_builds`(0、1、500 条目都能构造,不 panic);`overflow_menu_titles_are_clipped_not_wrapped`(超长标题的文本组件 `wrapping` 为 None、`clip` 为真,以构造出的样式断言)。
- [ ] **Step 2:** 运行确认失败。
- [ ] **Step 3:** 搬实现;`crate::app::*` 依赖改为 Task 2 的 byteui 函数;Dozer 专属的三个预览按钮不搬。
- [ ] **Step 4:** byteui 全量测试通过;`cargo build` 非 macOS 目标至少 `cargo check` 通过(能交叉则交叉,不能则在报告里写"未验证")。
- [ ] **Step 5:** 提交。

### Task 5: byteui 发版与 Dozer 重钉

**Files:**
- Modify: byteui `Cargo.toml` 版本、`CHANGELOG.md`;Dozer `Cargo.toml`/`Cargo.lock`;删除 Dozer `chrome/menu.rs`、`tab_widget.rs` 中已迁出的重复实现

- [ ] **Step 1:** byteui 版本递增(0.4.x → 0.5.0,因为新增模块属于 API 追加但对下游是新 minor,**此为我的决定**,可由用户改),CHANGELOG 写明迁入的模块。
- [ ] **Step 2:** **请求用户确认**后推送 byteui 并打 tag。
- [ ] **Step 3:** Dozer 改 `byteui = { tag = ... }` 为新 tag;`chrome/menu.rs`、`chrome/tab_widget.rs` 缩成重导出;**调用点不动**。
- [ ] **Step 4:** Dozer 门禁:全 workspace 测试数与迁移前相同(记基线数字)且 0 失败;`check-i18n.sh`、`check-log-scope.sh`、`check-shared-deps.sh`;`cargo tree -d` 里 byteui/iced 各 1 份。
- [ ] **Step 5:** Dozer 发新 libs tag(`libs-YYYY-MM-DD`);**请求用户确认**后推送 main 与 tag。

### Task 6: Digger 同日重钉并消费

**Files:**
- Modify: Digger `Cargo.toml`/`Cargo.lock`;删除 `adapters/menu_overlay.rs`;`adapters/agent_tabs.rs` 缩成映射;`adapters/native_menu.rs`、`product.rs` 等引用 `menu_overlay` 的调用点
- Test: Digger 现有测试(含 `agent_tabs`、菜单相关)

- [ ] **Step 1:** 重钉 byteui 与 Dozer libs tag;先让现有测试在**不改代码**时保持 0 失败(证明重钉本身没破坏)。
- [ ] **Step 2:** `menu_overlay.rs` 的调用点改指 `byteui::interaction::menu`,删除该文件;`agent_tabs.rs` 里与 byteui 重复的部件删除,只保留 Digger 的"库显示参数 → byteui 部件"映射。Digger 现有测试一条不删,只改路径。
- [ ] **Step 3:** 若 Task 1 裁决收敛"悬停阶跃 vs 渐变"差异,在此落实并补测试;否则保持原样并在 open-items 里保留。
- [ ] **Step 4:** Digger 干净快照整套门禁:build/test/clippy/fmt、`check-host-deps.sh`(含 `--self-test`)、`check-shared-deps.sh`、`check-shell-regions.sh`、`Cargo.lock` 重复检查。
- [ ] **Step 5:** 提交;更新 ADR-015 与 `agent-parity.md`,把"技术债"条目改成已解决。

---

# 阶段 B:bytehost-desktop(可信原生子 webview 执行器)

### Task 7: 把 `native_host.rs` 搬进 `bytehost-desktop::native_child`

**Files:**
- Create: `bytehost/crates/bytehost-desktop/src/native_child.rs`;Modify: `bytehost-desktop/src/lib.rs`
- Test: 同文件

**Interfaces:**
- Produces(从 Digger `native_host.rs` 公开面搬出,可见性 `pub(super)` → `pub`):`Desired { identity: String, url: String, rect: LogicalRect, scale: f64, visible: bool, loaded: bool }`、`Document { scheme: &'static str, host: &'static str, desired: Desired }`、`Action { Destroy, Create, Navigate, Zoom, SetBounds, Park, Show, … }`(以源文件为准)、`fn plan(current: Option<&Desired>, desired: Option<&Desired>) -> Vec<Action>`、`fn allows_navigation(scheme: &str, host: &str, url: &str) -> bool`、`fn allows_download() -> bool`(恒 false)、`fn new_window_response() -> wry::NewWindowResponse`(恒 Deny)、`Host`、`Probe`。`Host` 里与 iced `Task`/`window` 绑定的部分留在 bytehost-desktop,不带任何产品类型。

- [ ] **Step 1:** 先把 Digger 现有关于 `native_host` 的测试(`group_chat_native_tests.rs` 里 `assert!(!native_host::allows_download())` 等)逐条抄成 bytehost 里的失败测试:`plan_replaces_on_identity_change`、`plan_navigates_in_place_on_url_change`、`allows_navigation_only_for_the_trusted_origin`、`downloads_are_never_allowed`、`new_windows_are_always_denied`。
- [ ] **Step 2:** 运行确认失败(模块不存在)。
- [ ] **Step 3:** 搬实现;**不改任何 wry 设置的值**。迁移时 diff 搬前搬后的 builder 配置,逐项写进提交说明。
- [ ] **Step 4:** 增加源码钉死测试 `native_child_pins_the_restrictive_wry_settings`(读 `native_child.rs` 生产源码,断言下载、新窗口、开发者工具、媒体权限等每项仍显式关闭),并确认它**能失败**:临时改一项值,测试变红,再恢复。
- [ ] **Step 5:** `cargo test -p bytehost-desktop` 通过;bytehost 全 workspace 测试通过。提交。

### Task 8: bytehost 发版,Dozer 与 Digger 同日重钉

- [ ] **Step 1:** bytehost 版本递增(`v0.4.12` → `v0.4.13`,仅追加模块;此为我的决定),**请求用户确认**后推送并打 tag。
- [ ] **Step 2:** Dozer 重钉 bytehost(它不使用新模块,重钉只为两边钉版一致)、门禁全绿、发新 libs tag、**请求确认**后推送。
- [ ] **Step 3:** Digger 同日重钉,`check-shared-deps.sh` 0 阻断。

### Task 9: Digger 四个原生面板与 markdown 宿主改为消费

**Files:**
- Modify: `adapters/{usage_native,todo_native,group_chat_native,review_native}.rs`、`markdown/host.rs`;删除 `adapters/native_host.rs`
- Test: 各自现有测试

- [ ] **Step 1:** 四个原生面板的 `use super::native_host::…` 改为 `bytehost_desktop::native_child::…`;先让现有测试 0 失败。
- [ ] **Step 2:** `markdown/host.rs` 自带一份 wry builder 与源码钉死测试(读自身源码):评估并入 `native_child`。**若并入,钉死测试改读新位置并保证仍能失败**;若它的设置与 `native_child` 不同(它有 `with_new_window_req_handler`),保持独立并在报告里写明原因。
- [ ] **Step 3:** 删除 `native_host.rs`;Digger 干净快照整套门禁(同 Task 6 Step 4)。
- [ ] **Step 4:** 提交;更新 Digger open-items 与 ADR。

---

# 收口

### Task 10: 文档与记忆

- [ ] **Step 1:** Digger `notes/2026-10-18-conversations-open-items.md`、`group-chat-open-items.md` 的"仍然未决 X1"改为已解决(阶段 A、B 各自完成时分别更新)。
- [ ] **Step 2:** 本目录 `2026-10-28-s7-s8-remaining.md` 的 R7 标完成;`2026-10-27-s7-s8-closeout.md` §5 加指向本计划。
- [ ] **Step 3:** Dozer `CLAUDE.md` 的 byteui / bytehost 版本号与"已迁到共享库"的描述更新;记忆索引新增一条并更新"当前版本组"那条记忆。

---

## 范围外(明确不做)

- Dozer 自己的 wry 构建点(`runtime.rs` 三处、`file_history_overlay.rs`、`edit_history_overlay.rs`)并入 bytehost-desktop:Dozer 走的是 webview 池与不同的安全模型,与 Digger 的"可信子 webview"不是同一个执行器,**不在本计划内**,需要时另立计划。
- 非 macOS 的功能回退(只要求能编译)。
- 升级 iced、重命名既有 byteui 公开项、改 wire 形状。

## 需要用户裁决的点(默认值都是我的决定,可改)

1. byteui 版本号:默认 0.4.x → **0.5.0**;bytehost 默认 **v0.4.13**。
2. Task 1 里 Digger 的两个已知外观差异(悬停阶跃、提示 1 秒精度)是否收敛成 Dozer 的渐变/精度:**默认不收敛**,保持 Digger 现状。
3. 阶段 A、B 是否连着做:**默认分开**,A 全部完成并验收后再开 B,两者互不依赖。
4. `markdown/host.rs` 是否并入 `native_child`(Task 9 Step 2):**默认以 diff 结果为准**,设置不同就不并。

## 顺序

阶段 A(Task 1–6)含两次共享库发版(byteui、Dozer libs)和一次同日重钉;阶段 B(Task 7–9)含两次发版(bytehost、Dozer libs)和一次同日重钉。我没有可靠的工时依据,不给估计。建议先做 Task 1(只出报告、不改代码),据它确认范围后再往下。
