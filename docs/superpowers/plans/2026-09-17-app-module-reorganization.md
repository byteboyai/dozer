# Dozer 代码结构重组 Implementation Plan（平铺文件目录化 + 超大文件拆分）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `dozer-app` 里平铺的 ~50 个 `src/*.rs` 按"域"聚合进子目录，并拆分行数过大的单文件（`app.rs` 13004 行、`main.rs` 3569 行、`workspace.rs` 5266 行，以及 5 个 >2000 行的 `extensions/*`）。**纯重构，不改变任何用户可见行为。**

**Architecture:** 代码库里已经存在两种可复用的既定模式，本计划分别对应用到不同文件上，注意它们不是同一回事：
1. **"主文件保留内容 + 同名子目录放子模块"**（`extensions.rs`+`extensions/`、`theme.rs`+`theme/`、`project.rs`+`project/`、`ssh.rs`+`ssh/sftp.rs`）——主文件本身还有实质内容，子目录只装被它 `pub mod` 出去的一部分。
2. **"整个文件被同名目录+`mod.rs` 取代"**（Phase 2/3 的 `app.rs`→`app/mod.rs`、Phase 4.1/4.2 的 `workspace.rs`/`preview.rs`）——原文件物理删除，`mod.rs` 只做转发。这个拓扑在本仓库不是首次出现，`code_editor/mod.rs`、`dozerd/src/transcripts/mod.rs`、`byteui` 几个 `*/mod.rs` 已经在用，但**跟 extensions.rs/theme.rs 那四个例子不是同一种拓扑**，不要混为一谈——真正要复用的先例是前者这几个 `mod.rs` 目录。Phase 4.4 的域目录（`chrome/`/`project/`/`assets/`；原计划还有 `editor/`，写计划后核实调用图判定站不住已取消，见 Task 4.4）用的是模式 1。

以及"每扩展自带 `Message`/`State`/`update`/`view`"的扩展化方向（`git_log`/`conversations` 两个试点已完成）。本计划把这个既定方向**做完、做均匀**：先抽平台胶水（`main.rs`），再拆核心 `app.rs`，最后把 5 个巨型扩展目录化。

**Tech Stack:** Rust workspace；iced 0.14；纯 `mod`/`use`/可见性搬移 + `impl` 块跨文件分布（Rust 允许同一类型的 `impl` 块分布在不同模块文件）。

**Spec:** 无独立 spec（本次是纯结构重构，需求真相源仍是 `docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`，本计划只改文件布局不改行为）。

---

## 现状诊断（问题与数据）

`dozer-app` 行数 >2000 的文件（2026-09-17 实测；门槛从 800 提到 2000——低于 2000 的大多是"确实该拆但拆完两三份都健康"的中等文件，2000 以上才是真正需要专门 Task 处理的巨石）：

| 文件 | 行数 | 问题 |
|---|---|---|
| `src/app.rs` | 13004 | 状态类型 + 130 变体 `Message` + ~200 方法 + `view` + 2260 行测试全塞一起 |
| `src/extensions/database.rs` | 5302 | 单一扩展未拆分 |
| `src/workspace.rs` | 5266 | Workspace 内核 + view + 测试 |
| `src/extensions/files.rs` | 4526 | 同上 |
| `src/extensions/todo.rs` | 3923 | 同上 |
| `src/main.rs` | 3569 | `main()` 单函数 ~2900 行（内嵌 webview pool + 窗口事件状态机） |
| `src/extensions/usage.rs` | 2322 | 聚合计算 + view |
| `src/preview.rs` | 2265 | — |
| `src/extensions/project.rs` | 2264 | 已裂出 `project/`，仍偏大 |

2000 行门槛以下、原先在 800 门槛下也在列的文件降级为"不必再拆"：`extensions/git_log.rs`（1937）、`extensions/browser.rs`（1883）、`extensions/ssh.rs`（1580，已裂出 `ssh/sftp.rs`）——按 Phase 4.4 归进 `chrome`/`extensions` 域目录即可，不需要 Phase 4.5 那种 state/update/view 内部再拆。`homespace.rs`/`rail.rs`（~1000）本来就是"次优先级，归域后视需要内拆"，门槛调整不影响它们；`webview_geometry.rs`（~1000）经 Task 4.4 核实调用图后判定不进任何域目录，留在顶层原地不动（见 Task 4.4 修订说明），不再套用"次优先级归域"这条。

其余 crate 相对健康：`dozer-core/protocol.rs`（1540）、`dozerd/server.rs`（1246）、`dozerd/transcripts/mod.rs`（1319）在 Phase 5 处理。

**已知的三个"门槛提到 2000 也救不了"的残留点**（提前记录，避免执行完后被当成意外新发现；第三个是执行完 Phase 1 后审阅才发现的，补记于此）：
- **`app/app.rs`**（Phase 3.4 收敛后，`struct App` + 生命周期/accessor/布局持久化 `impl` 块，2414–5042）预计仍有 **≈2628 行**，且这个数字已经刨掉了测试（测试在 Task 3.4 就单独迁入 `app/tests/`）——它不是"测试太多"的问题，是这个 `impl` 块本身没有对应 `update.rs`/`view.rs` 那种二次拆分,Phase 3 没安排。**（2026-09-17 实测更新：Task 3.4 的测试拆分实际执行时被跳过未做，测试整体留在了 `app/app.rs` 尾部，实际行数是 5148 行，远超这里预估的 2628——测试拆分留待后续单独处理，见 Phase 3 Task 3.4 的实际结果记录。）**
- **`workspace/state.rs`**（`struct Workspace` + 全部 `impl Workspace`，89–2531）预计仍有 **≈2443 行**，同样已经刨掉测试（`workspace/tests/`）。Phase 4.1 对 `workspace.rs` 的拆分只切出了 `view.rs`/`hook.rs` 两块边角，`impl Workspace` 本体没有像 `app.rs` 那样的 update/view 二次切分方案。
- **`platform/window_events.rs`**（Phase 1 事后修正，见该 Phase"实际结果"一节）：`enum Runner` + `impl Runner` + `impl ApplicationHandler<Message> for Runner` 三部分合并后实测 **2708 行**——这是把同一个 `Runner` 类型的两个 impl 块合到一起的自然结果（内聚性更好），但也超过了门槛，要压下去需要在这个文件内部再按"`on_window_event` 状态机" vs "`ApplicationHandler` 三个 trait 方法"切一层。
- 这三处如果要真正压到 2000 行以下，需要在各自 Phase 之外补新 Task（`app/app.rs` 补做 Task 3.4 的测试域拆分，`workspace/state.rs` 参照 `app.rs` 的 update/view 分离思路再拆一层，`window_events.rs` 按状态机/trait 实现切开）——**当前版本计划没有这些 Task**，是否补，等实施到那一步时再定。

---

## 目标目录结构

### dozer-app/src/（Phase 1–4 完成后）

```
src/
├── main.rs                    # 只留 mod 声明 + main() 入口骨架(<150 行)
├── platform/                  # 窗口/拖拽/文件选择 平台胶水（从 main.rs 抽出）
│   ├── mod.rs
│   ├── window.rs              # center_traffic_lights / topbar_drag_guard
│   ├── file_drag.rs           # FILE_DRAG_POSITION / FILE_DRAG_WINDOW / drop 追踪
│   ├── window_events.rs       # Ready::on_window_event 状态机
│   └── picker.rs              # pick_file_or_dir
├── runtime.rs                 # spawn_dozerd / run_operate / sync_webview_pool / FocusAlso
├── event.rs                   # clear / unique_command_event / HOVER_ANIM_INTERVAL
├── app/                       # ← 13004 行 app.rs 落成目录（Phase 2–3）
│   ├── mod.rs
│   ├── message.rs             # Message enum + popup 结构体
│   ├── state.rs               # PanelKind/HoverId/AppPage/.../ShellState
│   ├── layout.rs              # ShellLayout/PanelDims/Divider/... + zone/几何纯函数
│   ├── app.rs                 # struct App + bootstrap + accessor（impl 块①）
│   ├── update.rs              # update() + handler（impl 块②）
│   └── view.rs                # view() + 自由 view 函数（impl 块③ + fn）
├── workspace/                 # ← 5266 行 workspace.rs 落成目录（Phase 4）
├── preview/                   # ← preview.rs
├── term/                      # term_model / term_view / terminal
├── assets/                    # assets / fonts / clipboard_image
├── chrome/                    # topbar / rail / tab_widget / menu / native_menu / homespace
├── project/                   # 按调用图核实后收缩为 project_meta（详见 Task 4.4 修订说明）
├── code_editor/                # 保持原地，不再包一层 editor/（Task 4.4 修订说明：editor/ 取消）
├── conversation.rs / transcript.rs / webview_geometry.rs / delivery.rs / open_projects.rs / panel_layouts.rs
│                             # ← 原计划分进 project/editor 两个域，核实调用图后判定是内核级/跨域共享文件，留在顶层（panel_layouts.rs 最终并入 app/app.rs，见 Task 4.4）
├── settings.rs / keymap.rs / dialog.rs / frosted.rs / layout.rs / osc.rs
├── theme.rs + theme/          # 保持不变
└── extensions.rs + extensions/ # 内部按 Phase 4 目录化，project_scaffold.rs 并入 extensions/project/scaffold.rs、diff_render.rs 移到 extensions/ 顶层挨着 git_log.rs
```

---

## Global Constraints

- **在独立分支 `refactor/module-reorganization` 上开发,不要直接提交到 main**；每阶段结束、构建/测试/clippy/fmt 全绿后单独 commit；全部完成后再提请审阅合并。
- **纯重构，零行为变化**：只做 `mod`/`use`/可见性/`pub use` 搬移，不改任何函数体逻辑、不改任何消息语义、不新增/删除功能。每个 Phase 结束用 `cargo run -p dozer-app` 走一遍关键路径确认行为不变。
- **兼容层优先**：拆分 `app.rs`/`workspace.rs` 这类被全 crate 引用的文件时，先在目标模块 `mod.rs` 里加 `pub use` 别名，让旧路径（如 `crate::app::Message` → 新 `crate::app::message::Message` 经 `pub use` 后 `crate::app::Message` 依旧可用）继续生效，最后再统一收敛引用路径，避免一次改动跨 30 个文件爆炸。
- **`impl` 块可跨文件**：Rust 允许 `impl App { .. }` 分布在同 crate 不同模块文件（只要 `App` 在作用域内），这是拆分 `app.rs` 的技术支点；不引入 trait、不引入 `Box<dyn Fn>`、不引入宏。
- **行号只作参考**：拆分的行号边界会因为前面的搬移而漂移，定位时用函数/结构体的闭合大括号，不要只信行号。
- 每个 Task 结束都要 `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过。
- **协作节奏**：`app.rs`/`workspace.rs`/`extensions/*` 是这个仓库改动最频繁的热点文件，本计划要分十几次 Task 逐步合并。开工前跟其他并行开发协调好这段时间对这几个文件的改动节奏（暂停非必要改动，或接受期间要多次 rebase 处理冲突），不要假设分支能一路开到 Phase 4 结束都不冲突。
- **域目录分配按调用图,不按文件名联想**：Phase 4.4/4.5 把文件分进哪个域目录之前，先跑 `grep -rl "<模块名>::" crates/dozer-app/src/` 确认真实调用方，不要凭文件名"读起来像哪个域"拍板。判断规则：只有一个消费者的文件，跟那个消费者放在一起（比如只被 `git_log` 用的文件应该贴着 `git_log` 走，不该进一个共享域目录，隔着一层目录反而更难找）；被三个以上不相关域共用的文件，是真正的共享内核工具，留在顶层或单独一个 `shared`/`util` 类目录，不要硬塞进某一个域的名字里——`webview_geometry.rs` 自己的模块文档就写着"跟文件预览业务域无关，是历史命名遗留"，这类自带说明的文件尤其要先读文档再分类。具体分组结果和取消/改派的成员见 **Task 4.4** 的修订说明（"目标目录结构"一节的树状图里也同步标了简注）。

---

## Phase 1：`main.rs` → `platform/` + `runtime.rs` + `event.rs`

**目标：** 把 `main()` 里 ~2900 行的启动序列、`Runner` 枚举、`sync_webview_pool`、`on_window_event` 状态机从 `main.rs` 拆出，`main()` 收敛到 <150 行。这是纯搬移，零逻辑改动，作为"文件+目录共存"模式的第二个样板。

### Task 1.1：`event.rs` —— 事件小工具

**Files:**
- Create: `crates/dozer-app/src/event.rs`
- Modify: `crates/dozer-app/src/main.rs`

搬移（`main.rs` 现有行 423–507）：
- `const HOVER_ANIM_INTERVAL: Duration`（423）
- `fn clear<'a>(..)`（435）
- `fn unique_command_event(ch: char) -> Event`（483）

`main.rs` 里 `mod event;`，两处调用点改 `crate::event::clear` / `crate::event::unique_command_event`。

### Task 1.2：`platform/window.rs` + `platform/file_drag.rs` + `platform/picker.rs`

**Files:**
- Create: `crates/dozer-app/src/platform/mod.rs`、`platform/window.rs`、`platform/file_drag.rs`、`platform/picker.rs`
- Modify: `crates/dozer-app/src/main.rs`

搬移（`main.rs` 现有行）：
- `window.rs`：`center_traffic_lights`（73）、`static TOPBAR_CONTROL_HOVERED`（137）、`install_topbar_drag_guard`（157）
- `file_drag.rs`：`static FILE_DRAG_POSITION`（219）、`file_drag_position()`（225/230 两处重载）、`static FILE_DRAG_WINDOW`（241）、`install_file_drag_position_tracker`（278）
- `picker.rs`：`pick_file_or_dir`（364）

### Task 1.3：`runtime.rs` —— daemon 启动 + webview 池

**Files:**
- Create: `crates/dozer-app/src/runtime.rs`
- Modify: `crates/dozer-app/src/main.rs`

搬移：
- `fn spawn_dozerd()`（508）
- `fn run_operate(..)`（572）
- `struct FocusAlso`（597）、`struct UnfocusTargets`（629）
- `main()` 内的 `fn sync_webview_pool(..)`（main.rs:796，7 个参数：`window`/`pool`/`specs`/`allowed_files`/`review_snapshot`/`proxy`/`report_title`）——**这已经是个普通嵌套 `fn`，不是闭包，没有捕获外部变量**，参数早就全部显式声明。提升到 `runtime.rs` 是纯剪切+补 `pub(crate)`，不需要做"捕获转参数"的改写。7 个参数里没有两个相邻同类型，按 CLAUDE.md 关键裁决的判断标准不强制拆成参数结构体，保持现状即可。

（main.rs 内还有一处同类误解要避免：`clear_file_drag_hover`（main.rs:1011）看起来像闭包但也是普通关联函数，Task 1.4 一并提升时同理不用管"捕获"。）

### Task 1.4：`platform/window_events.rs` —— 窗口事件状态机

**Files:**
- Create: `crates/dozer-app/src/platform/window_events.rs`
- Modify: `crates/dozer-app/src/main.rs`

`main()` 内 `enum Runner { Loading(..), Ready { .. } }`（main.rs:696–779）+ `impl Runner`（999–2187，含 `clear_file_drag_hover`(1011) 和 `on_window_event(&mut self, event: &WindowEvent) -> bool`(1037)）整体搬进 `window_events.rs`，`Runner` 枚举定义随同迁移（`platform/mod.rs` 做 `pub use`）。**这块实测约 1490 行**（`enum Runner` ~83 行 + `impl Runner` ~1188 行），比早先估算的"~380–905 行(约 525 行)"大近 3 倍——是 Phase 1 里工作量最大的一个 Task，预留时间要按实测数字算，不要按旧估算。

`impl winit::application::ApplicationHandler<Message> for Runner`（2188 起，含 `resumed`/`user_event`/`window_event`，约 1360 行）是另一个独立 impl 块——**这条判断是错的，已经在实测中纠正**：写这份计划时以为它可以留在 `main.rs`（理由是"`impl` 跨文件不影响调用，不用管它"），但没意识到这样会让 `main.rs` 整体依旧是个大文件，跟 Phase 1 标题自己定的"main.rs 收敛"目标直接矛盾。**正确做法是这个 impl 块也要搬进 `window_events.rs`**，跟 `impl Runner` 合并——两者是同一个 `Runner` 类型的两半，本就该在一起，也是内聚性更好的选择。

### Task 1.5：收敛 `main()` 入口

`main.rs` 最终只保留：`mod` 声明、`pub fn main()`（调 `fonts`/`theme` 初始化、建 event loop/tokio/client、`block_on(build_app(..))`、`event_loop.run_app`）。`build_app` 若仍在 `main.rs` 内且较大，一并并入 `runtime.rs`。

### Task 1.6：编译 / 测试 / clippy / fmt / 人工验收 + commit

### Phase 1 实际结果（2026-09-17 执行 + 事后修正）

首次执行只搬了 `enum Runner` + `impl Runner`（Task 1.4 原计划范围），`impl ApplicationHandler<Message> for Runner` 按 Task 1.4 当时（错误）的措辞留在了 `main.rs`——结果 `pub fn main()` 函数本身确实缩到了 <50 行，但**整个文件仍有 1498 行**，因为那块约 1360 行的事件分发状态机还杵在原地，跟 Phase 1"main.rs 收敛"的目标没对上。审阅发现后已在独立 commit（`6063645`）里补上：把 `impl ApplicationHandler` 连同它依赖的 `TODO_POLL_INTERVAL`/`DRAG_REDRAW_INTERVAL`/`menu_edit_key` 三个辅助项一起搬进 `platform/window_events.rs`，跟 `impl Runner` 合并到一起，并把 `app/app.rs`/`app/update.rs` 两处跨文件引用改成 `crate::platform::window_events::` 路径。

最终实测行数：
- `main.rs`：**82 行**（`pub fn main()` 本身 + `mod` 声明 + 极少量 `use`），达标。
- `platform/window_events.rs`：**2708 行**（`enum Runner` + `impl Runner` + `impl ApplicationHandler<Message> for Runner` 三部分合并后的结果）——这是"整个 Runner 事件状态机"一个内聚概念被合到一起的自然结果，不是意外堆积；但它现在也超过了本计划自己定的 2000 行门槛，如果要压下去，需要在这个文件内部再按"`on_window_event` 状态机" vs "`ApplicationHandler` 三个 trait 方法"切一层——本次未做，留作后续可选项，跟 `app/app.rs`/`workspace/state.rs` 那两处"门槛提到 2000 也救不了的残留点"是同一类情况。

`cargo build/test（859 全过）/clippy/fmt` 全绿。

---

## Phase 2：`app.rs` 类型抽离（message / state / layout）

**目标：** 先把 `app.rs` 里**纯类型与纯函数**抽成三个文件，编译器全程护航、风险最低、收益最大（约省 2200 行 + 130 变体枚举独立成文件）。

### Task 2.1：`app/` 目录骨架 + `mod.rs`

**Files:**
- Create: `crates/dozer-app/src/app/mod.rs`、`app/message.rs`、`app/state.rs`、`app/layout.rs`
- Modify: `crates/dozer-app/src/main.rs`（`mod app;` 保持不变——`app.rs` 和 `app/mod.rs` 都能满足这个 `mod` 声明，这是 Rust 2018 edition 起就有的模块解析规则，不是 2024 才有的能力，无需改引用）
- **Delete（关键，容易漏）**：`crates/dozer-app/src/app.rs` 本身。`app.rs` 和 `app/mod.rs` **不能同时存在**——两者都能满足 `mod app;`，同时存在会被 `rustc` 判为模块路径歧义直接报错编译失败。原 `app.rs` 的内容要在 Task 2.2–2.5 逐步搬空后，物理删除这个文件（最迟不晚于 Task 2.5 结束）；过渡期间可以先把已搬空的部分留一个空文件，但 Task 2.5 收尾时必须删掉。

`app/mod.rs` 先做**纯转发**，保证旧路径 `crate::app::X` 全部继续可用，同时把原 `app.rs` 顶部说明 App/Workspace 拆分边界的模块级 `//!` 文档注释一并搬过来（这段信息量不小，别漏掉）：

```rust
//! （原 app.rs 顶部的模块级 //! 文档注释搬到这里）

mod message;
mod state;
mod layout;

pub use message::*;
pub use state::*;
pub use layout::*;
```

（`app.rs` 原有内容随后逐步迁入子文件；过渡期 `app/mod.rs` 里 `mod app;` 承载尚未迁走的 `struct App`/`impl App`，见 Task 2.5——注意子模块也叫 `app`，即 `crate::app::app::App`，靠 `pub use app::*;` 转发回 `crate::app::App`，命名上容易和外层的 `app` 模块搞混，写代码/审阅时留意别读串。）

### Task 2.2：`message.rs` —— `Message` 枚举 + popup 结构体

搬移 `app.rs` 现有行段：
- 1886–2359：`pub enum Message`（130 变体，含注释）
- 2360–2413：`struct ProjectLinkMenu` / `struct CategoryContextMenu` / `enum CategoryPickerTarget` / `struct CategoryPicker` / `struct TextInputMenu` / `struct DatabaseSourceMenu`

`message.rs` 顶部补 `use crate::extensions::{browser, conversations, database, files, todo, usage, project, ssh, git_log, search, footbar};` 等 `Message` 变体引用的子消息类型。

### Task 2.3：`state.rs` —— 状态类型

搬移 `app.rs` 现有行段：
- 108–344：`PanelKind`（+impl）、`TopbarButton`、`TextInputTarget`、`HoverId`、`HoverAnim`（+impl）
- 377–459：`AppPage`、`MaximizedPane`、`ZoneSide`、`WorkspaceSlot`、`Side`
- 989–1074：`ShellState`
- 2741–2780：`ensure_project_readme`、`EXIT_TASK_BUDGET`（此两项属自由函数/常量，随 `struct App` 归 `app/app.rs`，见 **Task 2.5**；此处不搬）

### Task 2.4：`layout.rs` —— 布局/几何类型与纯函数

搬移 `app.rs` 现有行段：
- 440–985：`ShellLayout`（+Default）、`PanelDims`、`default_panel_dims`、`sanitize_shell_layout`、`sanitize_panel_dims`、`Divider`、`RowDivider`、`TabGroup`、`TabDrag`、拖拽阈值常量与判断函数、`webview_hidden_by_panel_popup`
- 1075–1700：`pair_list_content_width`、`pair_columns_tests`、`preview_desired_concurrent_tests`、`list_rendered_first`、`pair_split_ratio`、`with_pair_split_ratio`
- 1701–1885：`zone_at_x`、`terminal_pane_pixel_size`、`ssh_terminal_pane_pixel_size`、`UI_ZOOM_STEP`

**注意，不要把四个菜单项构造器（`project_link_menu_items`(822)/`text_input_menu_items`(839)/`database_source_menu_items`(887)/`category_context_menu_items`(927)，虽然物理位置落在 440–985 区间内）一起搬进 `layout.rs`**——它们分别构造 `Message::Project`/`Message::TextInputMenuCut` 等/`Message::Database`/`Message::Todo`，是四个不相关域各自的原生菜单数据，只是因为原文件里跟 `ShellLayout` 挨得近才显得像"布局"的一部分，塞进一个通用几何文件会让 `layout.rs` 变成跨四个域的杂物抽屉。这四个函数唯一的调用方分别是 `database_source_context_menu`(4511)/`project_link_context_menu`(4533)/`todo_category_context_menu`(4567) 三个方法（`text_input_menu_items` 的调用点在别处，同类性质），这三个方法本身落在 2796–5042 区间，属于 Task 2.5/3.4 最终收敛进 `app/app.rs` 的那块——所以这四个菜单构造器应该跟它们的调用方一起留在 `app/app.rs`，不要经过 `layout.rs` 中转。

### Task 2.5：`app/app.rs` —— `struct App` + `impl App` 主体迁入

`app/mod.rs` 里 `mod app;` 承载 `app.rs` 原 2414–9343 的 `struct App` + `impl App`（这一阶段**不拆 impl**，先整体平移，保持 `app/mod.rs` 的 `pub use app::*;` 让 `crate::app::App` 路径不变）。剩余自由 view 函数（9344–10743）与测试（10744–13004）暂留原 `app.rs`，本阶段先并入 `app/app.rs` 尾部，避免文件碎片化。另外把 Task 2.4 特意排除在 `layout.rs` 之外的四个菜单项构造器（`project_link_menu_items`/`text_input_menu_items`/`database_source_menu_items`/`category_context_menu_items`）也带过来，跟它们各自唯一的调用方（`database_source_context_menu`/`project_link_context_menu`/`todo_category_context_menu` 等）放在同一个文件里。

### Task 2.6：编译 / 测试 / clippy / fmt / 人工验收 + commit

---

## Phase 3：`app.rs` `impl App` 拆分（update / view）

**目标：** 用"`impl` 跨文件"技术把 ~6500 行的 `impl App` 切成 `app.rs`（生命周期+accessor）、`update.rs`（update+handler）、`view.rs`（view+自由函数），测试独立。

### Task 3.1：`app/view.rs` —— view 层

把 `impl App` 里的 `pub fn view(..)`（8712–9343）+ 自由 view 函数（9344–10743：`winning_agent_state`/`agent_state_dot`/`stub_activity`/`panel_body`/`left_panel_area`/`right_panel_area`/`maximize_overlay`/`ssh_tab_bar`/`ssh_tab_overflow_popup`/`ssh_terminal_pane`/`ssh_empty_state`）迁入 `app/view.rs`，写成独立的 `impl App { pub fn view(..) .. }` 块 + 同模块自由函数。

### Task 3.2：`app/update.rs` —— update + handler

把 `pub fn update(..)`（5042–6697）+ 全部 `fn xxx(..)` handler（6697–8712）迁入 `app/update.rs`，写成独立 `impl App { .. }` 块。`update()` 内 1650 行的 `match` 已按子域归组（project/database/ssh/todo/browser/terminal/preview），后续（Phase 3.3 可选）可按子域拆成 `update::database` 等子模块：把每个子域分支抽成私有 handler 方法，方法体放对应子文件。

### Task 3.3：（可选，后续单独 plan）`update/` 子域目录

`update()` 按 `database`/`ssh`/`todo`/`browser`/`terminal`/`preview`/`project` 抽 handler 方法到 `app/update/{domain}.rs`，每个文件 300–800 行。**本 Phase 只做 3.1/3.2，3.3 视后续工作量决定是否独立成 plan。**

### Task 3.4：`app/app.rs` 收敛 + 测试迁出

`app/app.rs` 保留：`struct App` + 生命周期/accessor/布局持久化的 `impl` 块（2796–5042）。测试（10744–13004，约 2260 行）迁入 `app/tests/`，按被测域拆成 `state.rs`/`layout.rs`/`update.rs`/`view.rs` 各自的 `#[cfg(test)] mod tests`。

这是全篇唯一一处没给具体行号切分的 Task，执行时**逐个测试函数对照它实际调用/断言的对象**来分类（测哪个类型的方法/字段就归哪个域），不要按测试写在原文件里的物理顺序整段切——原文件里不同域的测试是穿插写的。切完每一份测试文件，**必须逐条跟对应域最终迁移过去的参考实现代码交叉核对断言方向**，不能只看测试本身"读起来对不对"就通过：`docs/superpowers/plans/2026-08-07-files-extension-pilot.md` 那次的教训是计划阶段新增的测试断言方向写反，实现者照着（错的）测试把两处真实行为改错，代码审查才发现——这里测试量更大，重蹈的风险更高，Task 3.5 的人工验收要专门过一遍这批被迁移的测试用例本身有没有断言方向被搬串。

### Task 3.5：编译 / 测试 / clippy / fmt / 人工验收 + commit

---

## Phase 4：`workspace.rs` + `preview.rs` + 扩展目录化

### Task 4.1：`workspace/` 目录

`workspace.rs`（5266）落目录：
- `workspace/state.rs`：`ProjectRestore`/`RestorePayload`/`ReviewSource`/`ReviewView`/`SshOut`/`TabBackend`/`SessionTab`/`ShellIo`/`struct Workspace` + `impl Workspace`（89–2531）
- `workspace/view.rs`：`load_more_button`/`work_content_and_workspace_row`/`find_field_shell`/`preview_pane_for`（2532–3855）
- `workspace/hook.rs`：`should_summarize_on_close`/`should_answer_dynamic_color`/`dozer_hook_binary_path`/`ensure_hook_installed`/`ensure_mcp_installed`/`preview_context_from_editor_state`（3856–4204）
- `workspace/tests/`：`mod tests`（4205–5266）
- `workspace/mod.rs`：`pub use` 转发，保证 `crate::workspace::Workspace` 等旧路径不变；同时把原 `workspace.rs` 顶部模块级 `//!` 文档注释（如果有）搬过来。
- **Delete**：`crates/dozer-app/src/workspace.rs` 本身——同 Task 2.1 的 `app.rs` 一样，`workspace.rs` 和 `workspace/mod.rs` 不能同时存在，内容搬空后要物理删除原文件，不能只留着不管。

**内聚缺口（跟 Phase 3 对 `app.rs` 的处理不对称，先记录，不在本 Task 强制做）**：`workspace/state.rs` 是本计划里唯一一处"只挖走边角、`impl` 本体不拆"的地方——`struct Workspace` + 全部 `impl Workspace`（89–2531，≈2443 行）整体平移，没有对应 `app.rs` 那种 update/view 二次切分。这个 `impl` 块大概率能按子域再拆（终端会话页签生命周期 `spawn_new_tab`/`close_tab`/`attach` 一组、SSH 终端会话一组、`ws.review` 审阅内容加载一组、`from_restore`/`adopt_project`/`loading_for_project` 项目恢复重建一组——`hook.rs`/`view.rs` 已经把 hook 安装和纯 view 函数摘出去了，剩下的就是这几类），可以照 Task 3.1–3.2 的思路（先分类、再一个个搬）做一个 `workspace/tabs.rs` + `workspace/ssh_session.rs` + `workspace/review.rs` + `workspace/restore.rs` 之类的二次拆分。**跟 Task 3.3 一样标为可选**：本 Task 只做到 state/view/hook/tests 四分，二次拆分视工作量决定是否补一个 Task 4.1b 或独立成后续 plan——但不要在执行完 Task 4.1 后就当作"workspace.rs 已经处理完"，它是这次重组里内聚提升最不彻底的一块。

### Task 4.2：`preview/` 目录

`preview.rs`（2265）落目录，按 `state.rs`（preview 状态结构）/`native_editor.rs`（可编辑原生编辑器）/`webview.rs`（webview spec）/`view.rs` 切分。同样要把模块级文档注释搬进 `preview/mod.rs`，并在内容搬空后**删除原 `crates/dozer-app/src/preview.rs`**（原因同上，`preview.rs` 与 `preview/mod.rs` 不能共存）。

### Task 4.3：`term/` 目录（聚合并内拆 term_model/term_view/terminal）

`term_model.rs`（787）/`term_view.rs`（703）/`terminal.rs`（301）三个小文件聚合进 `term/`，`term/mod.rs` 转发。

### Task 4.4：`assets/`、`chrome/`、`project/` 域目录（纯聚合，不改文件内部）；`editor/` 取消

写这份计划时 `project/`/`editor/` 两个分组是按文件名联想拼的，写计划后用 `grep -rl "<模块名>::" crates/dozer-app/src/` 实测调用方，发现分组跟真实依赖对不上，按 Global Constraints 的"域目录分配按调用图"原则重新分：

- `assets/`：`assets.rs` + `fonts.rs` + `clipboard_image.rs`（未重新核实，按原计划）
- `chrome/`：`topbar.rs` + `rail.rs` + `tab_widget.rs` + `menu.rs` + `native_menu.rs` + `homespace.rs`（未重新核实，按原计划）
- `project/`：只留 `project_meta.rs`（调用方：`app.rs`/`extensions/project.rs`/`workspace.rs`，多消费者且含 `extensions::project`，是真正的"project 域"共享类型）。原计划里的另外三个成员改派：
  - `project_scaffold.rs`：**唯一调用方是 `extensions::project`**，不进 `project/`，改挪进 **`extensions/project/scaffold.rs`**（该目录已有 `delete.rs`/`links.rs`，正好配套），比隔着一层 `crate::project` 更贴近实际依赖，也避免跟 `extensions::project` 撞名造成误读。
  - `open_projects.rs`：唯一调用方是 `app.rs`（内核，管的是"跨项目的最近打开列表"，不是单个 project 内部状态），留在内核层，不进任何域目录。
  - `panel_layouts.rs`：唯一调用方也是 `app.rs`，且内容就是"面板布局持久化"——跟 Task 3.4 已经规划进 `app/app.rs` 的"生命周期/accessor/布局持久化"impl 块是同一件事，建议直接并入 `app/app.rs`，不单独成域。
  - `project.rs`（文件树状态机本体）：调用方除 `workspace.rs`/`main.rs` 外还有 `extensions/files.rs` **和** `extensions/ssh/sftp.rs`——不止服务"project"域，是文件树的通用抽象，被 Files 和 SSH/SFTP 两个不相关的面板复用。是否该叫 `project/`（会跟 `extensions::project` 撞名）值得重新命名（比如 `file_tree/`），具体分配到执行这个 Task 时再核实一遍，不要照抄本段的推测。
  - `git_watch.rs`：调用方是 `app.rs`/`workspace.rs`，跟同样是 git 工具的 `delivery.rs`（见下）性质一致，建议两个放一起，不归进 `project/`。
- **`editor/` 域整体取消**，原计划的六个成员逐一核实后没有一个站得住："跟 `code_editor` 放一起"这个理由本身就是按名字联想，不是按调用关系：
  - `diff_render.rs`：**唯一调用方是 `extensions::git_log`**，改挪进 `extensions/git_log.rs` 所在位置（Task 4.5 里 `git_log.rs` 已经在 2000 行门槛下判定"不用再拆"，`diff_render.rs` 直接留在 `extensions/` 顶层跟 `git_log.rs` 相邻即可，不用为它单独开子目录）。
  - `conversation.rs`：调用方是 `extensions::conversations`/`extensions::usage`/`homespace.rs`——没有一个是"编辑器"，是 Conversations 和 Usage 两个扩展共享的会话元数据类型，留在内核顶层（`src/conversation.rs` 原地不动），不并入任何域目录。
  - `transcript.rs`：调用方是 `app.rs`/`workspace.rs`，内容是"会话审阅"用的 `ReviewEntry` 适配器（跟 `conversation.rs` 是同一类"会话/审阅"数据，不是编辑器）。同样留在内核顶层原地不动。
  - `delivery.rs`：内容其实是 git CLI 薄封装（`repo_root`/分支/脏标），调用方横跨 `extensions/files.rs`、`extensions/git_log.rs`、`extensions/project.rs`、`homespace.rs`、`project_scaffold.rs`、`workspace.rs`、`app.rs`——不是"编辑器"，是被四五个不相关模块共用的 git 工具，建议跟 `git_watch.rs` 放一起（不强求进哪个域目录，两者都留顶层也可以，只要别进 `editor/`）。
  - `webview_geometry.rs`：文件自己的模块文档已经写明"跟文件预览业务域本身无关，是历史命名遗留"，调用方只有 `app.rs`/`main.rs`（内核），留在内核顶层原地不动，这条其实原计划的诊断表也该早点读到。
  - `code_editor/`：本来就是独立目录，本 Task 之前它已经在 `src/code_editor/`，不需要改动，也不需要为它单独包一层 `editor/`。

结论：Task 4.4 缩成 `assets/`/`chrome/`/`project/`（只剩 `project_meta.rs` 一个成员，规模很小，可以考虑跟 Task 4.4 一起顺手评估是否值得单独开目录，还是直接留在顶层）三个域目录，`editor/`/`git_watch.rs`+`delivery.rs` 这一组、`project.rs`(文件树) 的归属留给执行时用 `grep -rl` 现查现定，不要沿用本计划最初的猜测分组。

每域 `mod.rs` 用 `pub use` 做兼容层，再统一收敛 `app.rs`/`workspace.rs` 里的引用路径（`crate::topbar` → `crate::chrome::topbar`）。

### Task 4.5：`extensions/` 5 个巨型扩展目录化

照 `git_log`/`conversations` 两个已完成试点，逐一把以下 >2000 行的扩展落目录（`mod.rs` 转发，`Message`/`State`/`update`/`view` 各自成文件）。`git_log.rs`（1937）/`browser.rs`（1883）/`ssh.rs`（1580）在 2000 行门槛下不再要求内部再拆，按 Task 4.4 归进域目录（`extensions/` 原地不动即可）就算完成：

| 扩展 | 目标子文件 |
|---|---|
| `database.rs`（5302） | `state.rs`(DataSource/TableRef/SchemaState/Draft) / `load.rs`(SQL+spawn) / `update.rs` / `view.rs` |
| `files.rs`（4526） | `state.rs` / `tree.rs`(TreeDrag/drop target) / `update.rs` / `view.rs` |
| `todo.rs`（3923） | `state.rs` / `filter.rs`(纯过滤/分类) / `update.rs` / `view.rs` |
| `usage.rs`（2322） | `aggregate.rs`(纯聚合计算) / `chart.rs` / `view.rs` |
| `project.rs`（2264，已裂 delete/links） | 续拆 `scaffold.rs` / `update.rs` / `view.rs` |

每个扩展一个 Task，独立 commit，独立跑 `cargo run` 验收该面板行为不变。

### Task 4.6：全量校验与人工验收

`cargo build && cargo test -p dozer-app && cargo clippy --all-targets && cargo fmt --check` 全绿；`cargo run -p dozer-app` 走一遍文件树/预览/终端/数据库/SSH/Todo/Usage/Git Log/Browser/会话面板关键路径。

---

## Phase 5（后续单独 plan，不在本计划内展开）：dozerd / dozer-core

- `dozer-core/src/protocol.rs`（1540）→ `protocol/`：`agent.rs` / `request.rs` / `response.rs` / `info.rs`（按 `AgentKind`/请求/响应/`*Info` 类型族拆）
- `dozerd/src/server.rs`（1246）→ `server/`：按 `preview_context`/`todo`/`session`/`summary` handler 分组
- `dozerd/src/transcripts/mod.rs`（1319）→ 内部按扫描/解析/派发拆
- `dozerd/src/ide_bridge.rs`（985）：如继续增长再拆

---

## 实施顺序与风险控制

1. **先地基后楼宇**：Phase 1（`main.rs`）纯搬移，建立"文件+目录共存"第二个样板，验证兼容层写法。
2. **再拆 `app.rs`**：先 Phase 2 类型抽离（纯类型，编译器全程护航），再 Phase 3 `impl` 拆分（`view.rs` → `update.rs` 顺序，因为 view 依赖少、边界清晰）。
3. **最后域聚合 + 扩展目录化**：Phase 4 逐步进行，每步一个 commit。
4. 每 Phase 结束跑 `cargo run -p dozer-app` 人工确认行为不变；`pub use` 兼容层保留到全 crate 引用路径收敛完成后才移除。
5. 全绿后提请审阅，审阅通过才合并 `refactor/module-reorganization` 到 `main`。
