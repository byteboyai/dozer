# Dozer 代码结构重组 Implementation Plan（平铺文件目录化 + 超大文件拆分）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `dozer-app` 里平铺的 ~50 个 `src/*.rs` 按"域"聚合进子目录，并拆分行数过大的单文件（`app.rs` 13004 行、`main.rs` 3569 行、`workspace.rs` 5266 行，以及 5 个 >2000 行的 `extensions/*`）。**纯重构，不改变任何用户可见行为。**

**Architecture:** 代码库里已经存在两种可复用的既定模式，本计划分别对应用到不同文件上，注意它们不是同一回事：
1. **"主文件保留内容 + 同名子目录放子模块"**（`extensions.rs`+`extensions/`、`theme.rs`+`theme/`、`project.rs`+`project/`、`ssh.rs`+`ssh/sftp.rs`）——主文件本身还有实质内容，子目录只装被它 `pub mod` 出去的一部分。Phase 4.4 的域目录（`chrome/`/`project/`/`editor/`/`assets/`）用的是这个模式。
2. **"整个文件被同名目录+`mod.rs` 取代"**（Phase 2/3 的 `app.rs`→`app/mod.rs`、Phase 4.1/4.2 的 `workspace.rs`/`preview.rs`）——原文件物理删除，`mod.rs` 只做转发。这个拓扑在本仓库不是首次出现，`code_editor/mod.rs`、`dozerd/src/transcripts/mod.rs`、`byteui` 几个 `*/mod.rs` 已经在用，但**跟 extensions.rs/theme.rs 那四个例子不是同一种拓扑**，不要混为一谈——真正要复用的先例是前者这几个 `mod.rs` 目录。

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

2000 行门槛以下、原先在 800 门槛下也在列的文件降级为"不必再拆"：`extensions/git_log.rs`（1937）、`extensions/browser.rs`（1883）、`extensions/ssh.rs`（1580，已裂出 `ssh/sftp.rs`）——按 Phase 4.4 归进 `chrome`/`extensions` 域目录即可，不需要 Phase 4.5 那种 state/update/view 内部再拆。`homespace.rs`/`rail.rs`/`webview_geometry.rs`（~1000）本来就是"次优先级，归域后视需要内拆"，门槛调整不影响它们。

其余 crate 相对健康：`dozer-core/protocol.rs`（1540）、`dozerd/server.rs`（1246）、`dozerd/transcripts/mod.rs`（1319）在 Phase 5 处理。

**已知的两个"门槛提到 2000 也救不了"的残留点**（提前记录，避免执行完后被当成意外新发现）：
- **`app/app.rs`**（Phase 3.4 收敛后，`struct App` + 生命周期/accessor/布局持久化 `impl` 块，2414–5042）预计仍有 **≈2628 行**，且这个数字已经刨掉了测试（测试在 Task 3.4 就单独迁入 `app/tests/`）——它不是"测试太多"的问题，是这个 `impl` 块本身没有对应 `update.rs`/`view.rs` 那种二次拆分,Phase 3 没安排。
- **`workspace/state.rs`**（`struct Workspace` + 全部 `impl Workspace`，89–2531）预计仍有 **≈2443 行**，同样已经刨掉测试（`workspace/tests/`）。Phase 4.1 对 `workspace.rs` 的拆分只切出了 `view.rs`/`hook.rs` 两块边角，`impl Workspace` 本体没有像 `app.rs` 那样的 update/view 二次切分方案。
- 这两处如果要真正压到 2000 行以下，需要在 Phase 3/4.1 之外补一个新 Task（比如 `app/app.rs` 再按"生命周期/accessor/布局持久化"三块拆，`workspace/state.rs` 参照 `app.rs` 的 update/view 分离思路再拆一层）——**当前版本计划没有这个 Task**，是否补，等实施到那一步时再定。

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
├── project/                   # project / project_meta / project_scaffold / open_projects / panel_layouts / git_watch
├── editor/                    # code_editor / diff_render / delivery / transcript / conversation / webview_geometry
├── settings.rs / keymap.rs / dialog.rs / frosted.rs / layout.rs / osc.rs
├── theme.rs + theme/          # 保持不变
└── extensions.rs + extensions/ # 内部按 Phase 4 目录化
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

`impl winit::application::ApplicationHandler<Message> for Runner`（2188 起，含 `resumed`/`user_event`/`window_event`）是另一个独立 impl 块，**不在本 Task 搬移范围内**，留在 `main.rs`（它调用 `on_window_event` 这个跨文件的关联函数即可，`impl` 跨文件不影响调用）。

### Task 1.5：收敛 `main()` 入口

`main.rs` 最终只保留：`mod` 声明、`pub fn main()`（调 `fonts`/`theme` 初始化、建 event loop/tokio/client、`block_on(build_app(..))`、`event_loop.run_app`）。`build_app` 若仍在 `main.rs` 内且较大，一并并入 `runtime.rs`。

### Task 1.6：编译 / 测试 / clippy / fmt / 人工验收 + commit

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
- 440–985：`ShellLayout`（+Default）、`PanelDims`、`default_panel_dims`、`sanitize_shell_layout`、`sanitize_panel_dims`、`Divider`、`RowDivider`、`TabGroup`、`TabDrag`、拖拽阈值常量与判断函数、`webview_hidden_by_panel_popup`、各菜单项构造器（`project_link_menu_items`/`text_input_menu_items`/`database_source_menu_items`/`category_context_menu_items`）
- 1075–1700：`pair_list_content_width`、`pair_columns_tests`、`preview_desired_concurrent_tests`、`list_rendered_first`、`pair_split_ratio`、`with_pair_split_ratio`
- 1701–1885：`zone_at_x`、`terminal_pane_pixel_size`、`ssh_terminal_pane_pixel_size`、`UI_ZOOM_STEP`

### Task 2.5：`app/app.rs` —— `struct App` + `impl App` 主体迁入

`app/mod.rs` 里 `mod app;` 承载 `app.rs` 原 2414–9343 的 `struct App` + `impl App`（这一阶段**不拆 impl**，先整体平移，保持 `app/mod.rs` 的 `pub use app::*;` 让 `crate::app::App` 路径不变）。剩余自由 view 函数（9344–10743）与测试（10744–13004）暂留原 `app.rs`，本阶段先并入 `app/app.rs` 尾部，避免文件碎片化。

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

### Task 4.2：`preview/` 目录

`preview.rs`（2265）落目录，按 `state.rs`（preview 状态结构）/`native_editor.rs`（可编辑原生编辑器）/`webview.rs`（webview spec）/`view.rs` 切分。同样要把模块级文档注释搬进 `preview/mod.rs`，并在内容搬空后**删除原 `crates/dozer-app/src/preview.rs`**（原因同上，`preview.rs` 与 `preview/mod.rs` 不能共存）。

### Task 4.3：`term/` 目录（聚合并内拆 term_model/term_view/terminal）

`term_model.rs`（787）/`term_view.rs`（703）/`terminal.rs`（301）三个小文件聚合进 `term/`，`term/mod.rs` 转发。

### Task 4.4：`assets/`、`chrome/`、`project/`、`editor/` 域目录（纯聚合，不改文件内部）

- `assets/`：`assets.rs` + `fonts.rs` + `clipboard_image.rs`
- `chrome/`：`topbar.rs` + `rail.rs` + `tab_widget.rs` + `menu.rs` + `native_menu.rs` + `homespace.rs`
- `project/`：`project.rs` + `project_meta.rs` + `project_scaffold.rs` + `open_projects.rs` + `panel_layouts.rs` + `git_watch.rs`
- `editor/`：`code_editor/`（并入）+ `diff_render.rs` + `delivery.rs` + `transcript.rs` + `conversation.rs` + `webview_geometry.rs`

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
