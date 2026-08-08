# `App`/`Workspace` 文件拆分 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `crates/dozer-app/src/workspace.rs`(拆分基准是首页四栏化分支
`feature/home-page-4col-layout` 合并进 `main` 之后的状态,约 8506 行)拆成
新文件 `app.rs`(`App` struct + `Message` 枚举 + `impl App` + App-only 视图
函数 + 6 个"壳层组装内容"的混合函数 + 壳层状态小类型)与瘦身后的
`workspace.rs`(`Workspace` struct + `ShellIo` + `impl Workspace` + 只依赖
`Workspace` 的自由函数)。

**Architecture:** 纯代码搬家 + 可见性调整,不改任何行为。依赖方向单向:
`app.rs` 依赖 `workspace.rs` 的 `Workspace` 类型(`use crate::workspace::
Workspace;`);`workspace.rs` 只反向依赖 `app.rs` 的 `Message` 一个类型
(`Workspace` 的异步方法要构造 `Message` 变体回传)。这个方向性已经被现状
代码验证过——`impl Workspace` 的 40 个方法里零处真实依赖 `App`,`ShellIo`
这个值类型的存在就是为了避免 `Workspace` 反向借用 `App`。

**Tech Stack:** Rust workspace;iced 0.14;不涉及新依赖。

## Global Constraints

- **在独立分支上开发**:建分支 `feature/app-workspace-split`(或
  `superpowers:using-git-worktrees` 建同名 worktree),完成后提请审阅,通过
  再合并回 `main`。**这个分支必须从"`feature/home-page-4col-layout` 已合并
  进 main"之后的 `main` 状态开工**——本计划的全部行号、清单都基于那个状态
  分析得出;如果你现在看到的 `main` 上 `workspace.rs` 还有 `home_sidebar`/
  `home_recents_column` 等函数(即首页分支还没合并),先停下来确认合并状态,
  不要在旧状态上套用本计划的清单。
- **行号会漂移,按符号名定位,不要死信行号**:本计划里给出的行号是写计划
  当下(对 `feature/home-page-4col-layout` 分支内容)分析得出的快照。如果
  你实际操作时 `main` 上 `workspace.rs` 已经因为其它无关改动有过变化,行号
  会对不上——**用 `grep -n "^fn NAME\|^pub fn NAME\|^pub(crate) fn NAME"`
  等按函数/类型名重新定位当前实际行号**,不要盲信本文档写死的数字。
- **这是纯代码组织重构,不改任何行为**——UI 观感、消息流转、状态语义、
  既有测试断言全部原样保留。不顺带重构任何方法实现、不合并/拆分任何既有
  方法、不改任何字段的语义、不改 `Message` 任何变体的名字或载荷类型。
- **搬迁顺序刻意设计成"先搬大块、后搬散件",每步都要能独立编译通过**:
  Task 1 先搬 `Message`/`App` struct/`impl App` 这三大块(而不是"分组搬 App
  相关的全部内容"),因为这三块是 `impl App` 唯一必须一起动的部分,其余
  App-only 自由函数留在 `workspace.rs` 原地不动、只加 `pub(crate)`,靠
  跨文件 `use` 保持编译通过——这样 Task 1 结束时整个 crate 已经是可编译、
  可测试的中间态,不是"改到一半崩着"的状态。Task 2 再把其余 App-only 自由
  函数挪过去,消掉 Task 1 里为了过渡加的那些 `pub(crate)`/跨文件 `use`(挪
  过去之后同文件访问不再需要它们,但留着也不算错,不强制清理,是否顺手清
  由实现者判断)。
- **分类清单是"数据驱动 + 编译器兜底"的产物,不是绝对真理**:本计划的
  App-side/Workspace-side 清单是通过脚本统计每个符号被 App 侧代码引用还是
  Workspace 侧代码引用生成的,人工核对过明显反常的案例,但**不保证 100%
  精确**。执行过程中如果编译器在某一步报错(比如某个"分到 workspace.rs"的
  函数其实被刚搬到 `app.rs` 的代码直接引用、缺一个 `pub(crate)` 或
  `use`),按报错提示直接修(加可见性/加 `use`,或者如果确实分错了侧,把
  那一个符号挪到另一个文件),不要因为一处分类的偏差就回头重新设计整个
  拆分方案。
- 每个任务结束都要 `cargo build && cargo test && cargo clippy --all-targets
  && cargo fmt` 干净通过(全 workspace)。
- 设计文档:`docs/superpowers/specs/2026-08-08-app-workspace-file-split-
  design.md`(有疑问以它为准)。

---

## 符号归属清单

以下按拆分基准(`feature/home-page-4col-layout` 合并后的 `workspace.rs`,
约 8506 行)分析得出。**"起始行"仅供搜索定位参考**,实际操作请按符号名
`grep` 现状行号。每一项的"结束行"= 该符号的花括号完整闭合处(含末尾
`}`)。项之间原本的空行/文档注释块随该项一起搬(即从它自己的 `///` 文档
注释块开头搬到闭合 `}` 为止,不留孤儿注释)。

### 归 `app.rs` 的符号(71 项)

**壳层状态小类型 + 几何纯函数(现有顺序,原样搬,不重新分组)**:

`DEFAULT_COLS`(78)、`DEFAULT_ROWS`(79)、`LeftView`(85-91,与
`RightView` 是同一对"左/右面板区当前显示哪个视图"的姊妹枚举,必须一起
搬,原分析脚本第一版有个 bug 漏掉了它,写计划过程中已核实修正——见本节
末尾说明)、`RightView`(95-100)、`RailButton`(106-122)、
`TopbarButton`(129-132)、`HoverId`(140-147)、`HoverAnim` struct
(153-158)、`impl HoverAnim`(159-181)、`AppPage`(186-190)、
`MaximizedPane`(194-197)、`ZoneSide`(204-207)、`WorkspaceSlot`
(212-225——虽然名字带"Workspace",但它是 `App.projects: HashMap<i64,
WorkspaceSlot>` 的值类型,被 `impl App` 大量使用、`impl Workspace` 零处
使用,归 `App` 一侧)、`ShellLayout`(340-359)、`impl Default for
ShellLayout`(361-376)、`sanitize_shell_layout`(386-418)、`Divider`
(423-427)、`ShellState`(434-446)、`zones_width`(452-455)、
`clamp_left_width`(469-473)、`left_zone_width`(478-486)、
`right_zone_width`(491-496)、`pair_content_width`(502-504)、
`apply_column_drag`(510-571)、`maximized_box_x_range`(581-588)、
`maximized_box_height`(594-599)、`preview_content_bounds`(610-689)、
`is_in_preview_column`(696-744)、`zone_at_x`(755-783)、
`terminal_visible`(790-794)、`terminal_grid_state`(803-819)、
`terminal_pane_pixel_size`(830-864)、`ProjectId`(type alias,单行,882)、
`UI_ZOOM_STEP`(单行,885,⌘+/⌘- 缩放步进常量,`Message::ZoomIn/
ZoomOut` 处理器用,窗口级概念)。

> **关于这份清单的可靠性**:写计划过程中用脚本(按顶层 `fn`/`struct`/
> `enum`/`impl`/`const`/`type` 声明 + 花括号配对)扫描全文件生成初版清单
> 时,发现脚本对不带花括号的 `const`/`type` 单行声明处理有 bug——如果
> 一个 `const`/`type` 紧挨着下一个带花括号的项,脚本会把两者错误合并成
> 一个"项",导致被合并进去的那个(这里是 `LeftView`)在初版清单里完全
> 消失。已修脚本重新跑过并逐行核对总数(121 个顶层项,清单两节加起来应
> 精确等于 121),下面两节的清单是修正后的版本。这个教训本身也提醒实现者:
> **如果执行过程中发现某个符号两边清单都没提到,大概率不是"不需要搬",
> 而是清单本身的遗漏——先按它实际的 `app`/`ws` 依赖关系判断该归哪边,
> 不要假设清单是穷尽的绝对真理**(这条本来就是 Global Constraints 里
> "编译器兜底"原则的具体例证)。

**`Message` 枚举 + `App` struct + `impl App`(三大块)**:

`Message`(897-1099)、`App` struct(1230-1324)、`impl App`(2344-4207,
1864 行,`crate` 里最大的一块)。

**App 拥有的项目页签/槽位管理自由函数**(操作 `App.projects`/
`App.project_order`/`App.active_project_id` 这几个 App 自己的字段,虽然
签名可能不显式带 `app: &App`,是因为它们是纯函数、调用方在 `impl App`
内部传字段进来,但语义上是 App 的私有辅助逻辑):

`focus_project_tab`(1398-1408)、`next_active_after_close`(1415-1421)、
`take_project_tab`(1426-1438)、`loaded_workspace_mut`(1446-1456)、
`restore_open_tabs`(1474-1498)。

**顶栏/项目页签渲染(App-only 自由函数)**:

`top_bar_font`(4358-4363)、`dozer_home_tab`(4372-4432)、`top_bar`
(4434-4514)、`project_tab_entries`(4519-4544)、`ProjectTabEntry`
(4547-4552)、`project_tabs_row`(4562-4672)、`project_tab_item`
(4676-4866)。

**项目页签状态点计算(App-only,虽然 `project_tab_dot` 签名带 `ws:
&Workspace`,但它唯一的调用方是 App-only 的 `project_tab_entries`,读
"这个已加载的 Workspace 该显示什么状态点"——跟 `stub_activity` 是同一组
"给 Stub/Loaded 两种槽位算状态点"逻辑,放一起不拆开)**:

`project_tab_dot`(4870-4878)、`project_dot`(4882-4884)、
`winning_agent_state`(4894-4903)、`agent_state_dot`(4907-4912)、
`stub_activity`(4922-4929)。

**图标栏/面板区组装(App-only + 6 个"混合组装"函数)**:

`rail_icon_button`(5322-5372)、`left_icon_rail`(5375-5441)、
`right_icon_rail`(5444-5536)、`PaneCorner`(5550-5555)、
`zone_pane_border`(5562-5584)、`worktree_strip`(5608-5679,唯一调用方
`left_panel_area` 是 App 侧)、`left_panel_area`(5681-5824,混合函数)、
`right_panel_area`(5838-5958,混合函数)、`maximize_overlay`(5963-6030,
混合函数)、`terminal_pane`(6308-6356,混合函数)、`divider_bar`
(6374-6419,拖拽分隔线,壳层拖宽状态机的一部分)、`tab_arrow_button`
(6605-6645,项目页签栏/预览页签栏共用的通用组件,两侧都要用,归 App 侧,
`workspace.rs` 侧的 `preview_pane` 调用它需要 `pub(crate)` + 跨文件
`use`)、`tab_divider`(6648-6658,同上)、`tab_bar`(6665-6708,混合函数)、
`tab_window`(6829-6857,同 `tab_arrow_button` 理由,两侧共用)、`tab_item`
(6913-6972,唯一调用方 `tab_bar` 是混合函数/App 侧)、`active_tab_view`
(6974-6989,混合函数)。

### 留在 `workspace.rs` 的符号(50 项,原地不动)

`ProjectRestore`(239-250)、`RestorePayload`(260-272)+ 它的 `impl Clone`
(274-278)+ `impl Debug`(280-284)、`PickerLaunch`(891-894,Agent 面板"＋"
弹出菜单选择项——`Agent(Option<AgentKind>)`/`Git`,消费方
`agent_picker_popup`/`picker_launch_command` 都在 `workspace.rs` 一侧,
虽然它也作为 `Message::AgentPickerSelect(PickerLaunch)` 的载荷类型被
`app.rs` 的 `Message` 引用——这和 `ReviewSource`/`AddrEvent` 是同一种"App
侧 `Message` 引用一个 Workspace 侧类型"模式,不需要为此把 `PickerLaunch`
搬去 `app.rs`)、`AddrEvent`(1104-1109)、
`ReviewSource`(1113-1116)、`review_should_refresh_on_turn`(1119-1121)、
`ReviewView`(1124-1130)、`EditSession`(1133-1144)、`SessionTab` struct
(1147-1178)、`effective_cwd`(1183-1188)、`impl SessionTab`(1190-1206)、
`ShellIo`(1218-1225)、`Workspace` struct(1326-1387)、`impl Workspace`
(1500-2309,810 行,原地不动)、`spawn_project_git_refresh`(2319-2342,
调用方是 `Workspace` 自己的生命周期方法)、`exited_marker`(4254-4262)、
`review_content`(4267-4335)、`conversation_list_pane`(4933-5049)、
`group_tabs_by_agent`(5057-5076)、`agent_list_pane`(5081-5132)、
`agent_list_row`(5138-5173)、`agent_picker_toggle_button`(5178-5198)、
`agent_picker_popup`(5207-5277)、`review_content_pane`(5282-5318)、
`split_portions`(5540-5544)、`tree_row_font_size`(6035-6037)、`lh`
(6042-6046)、`no_project_placeholder`(6052-6102)、`terminal_status_bar`
(6105-6137)、`status_bar_container`(6144-6164)、`preview_pane`
(6166-6305)、`edit_modal`(6425-6531)、`edit_discard_confirm_popup`
(6535-6602)、`relative_time_text`(6714-6725,`homespace.rs` 已经在用,
保持从 `crate::workspace::` 导入不变)、`conversation_sub`(6728-6736)、
`ai_turn_summary`(6739-6746)、`effective_project_repo`(6749-6753)、
`acceptance_query_repo`(6759-6761)、`agent_cli_command`(6769-6774)、
`picker_launch_command`(6780-6786)、`tab_title`(6790-6801)、
`text_width_units`(6805-6809)、`tab_display_width`(6813-6816)、
`preview_tab_display_width`(6819-6822)、`agent_state_label`(6860-6867)、
`dot_color`(6872-6881)、`agent_dot_color`(6885-6895)、`agent_icon`
(6899-6907)。

---

### Task 1: 建 `app.rs`,搬 `Message`/`App` struct/`impl App` 三大块

**Files:**
- Create: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/main.rs`(加 `mod app;`)
- Modify: `crates/dozer-app/src/workspace.rs`(删掉这三块,其余 68 项
  App-only 符号**这一步先不动**,只按需加 `pub(crate)`)
- Modify: `crates/dozer-app/src/homespace.rs`(`App`/`Message`/`HoverId`/
  `RailButton`/`PaneCorner`/`rail_icon_button`/`zone_pane_border` 的导入
  来源从 `crate::workspace::` 改成 `crate::app::`——这几个符号本身还没搬
  (Task 2 才搬),但 `App`/`Message` 这两个已经在这一步搬了,`homespace.rs`
  当下就要跟着改,否则编不过)

**Interfaces:**
- Produces:`app.rs` 里的 `pub struct App`、`pub enum Message`、`impl App`
  的全部现有公开方法(签名不变,原样搬)。
- Consumes:`crate::workspace::Workspace`(单向 `use`)、`crate::workspace::`
  里其余 68 项 App-only 符号(这一步它们还没搬走,`app.rs` 要
  `use crate::workspace::{符号列表};` 引用,workspace.rs 侧对应符号加
  `pub(crate)`)。

- [ ] **Step 1: 核对当前行号**

`main` 已合并首页分支的前提下,跑:

```bash
grep -n "^pub enum Message {" crates/dozer-app/src/workspace.rs
grep -n "^pub struct App {" crates/dozer-app/src/workspace.rs
grep -n "^impl App {" crates/dozer-app/src/workspace.rs
```

和本文档"符号归属清单"里 `Message`(897)/`App` struct(1230)/`impl App`
(2344)这三个起始行核对。如果对不上,说明 `main` 在首页分支合并后又有别的
提交动过 `workspace.rs`——以 `grep` 结果为准,后续步骤里所有行号按差值
平移。

- [ ] **Step 2: 创建 `app.rs`,写文件头**

```rust
// crates/dozer-app/src/app.rs
//! 整个程序只有一份的外壳态:daemon 客户端、窗口尺寸、图标栏/面板区的
//! 收起-拖宽-放大状态、并行打开的 N 个项目页签。`App::view()`/`update()`
//! 是 iced 应用的顶层入口,`Message` 是它的消息协议。
//!
//! 与 `Workspace`(单个项目的全部状态,见 `crate::workspace`)的拆分边界:
//! `impl Workspace` 的方法零处依赖 `App`(`ShellIo` 就是为此存在的解耦
//! 值类型),因此依赖方向是单向的——本文件 `use crate::workspace::
//! Workspace;`,`workspace.rs` 只反向借用本文件的 `Message` 一个类型
//! (`Workspace` 的异步方法要构造 `Message` 变体经 `EventLoopProxy`
//! 回传)。拆分细节见
//! `docs/superpowers/specs/2026-08-08-app-workspace-file-split-design.md`。
```

- [ ] **Step 3: 剪切 `Message`/`App`/`impl App` 三块到 `app.rs`**

按 Step 1 核对后的实际行号,把这三块**原样剪切**(不改一个字符)到
`app.rs`(紧接文件头之后,顺序:`Message` → `App` struct → `impl App`)。

- [ ] **Step 4: `app.rs` 加 `use`**

把 `workspace.rs` 原顶部的**全部** `use` 语句原样复制一份到 `app.rs` 顶部
(不用逐条甄别哪些用得上——`cargo build` 会把用不到的标成 unused import
警告,Step 8 统一按警告清掉,比手工甄别快且不会漏)。额外补两行:

```rust
use crate::workspace::Workspace;
```

（`ShellIo`/`WorkspaceSlot` 等其余 `impl App`/`App` struct 用到的
`workspace.rs` 符号,已经在复制过来的原 `use crate::...` 列表里覄盖不到的
话,这一步先用 `crate::workspace::{符号名}` 补,具体缺哪个由 Step 8 编译
报错驱动补全,不用现在穷举。)

- [ ] **Step 5: `main.rs` 加 `mod app;`**

`main.rs` 的 `mod` 列表按字母序,在 `mod assets;` 之后、`mod
clipboard_image;` 之前插入 `mod app;`。

- [ ] **Step 6: `workspace.rs` 侧收尾——补可见性,补跨文件 `use`**

`workspace.rs` 现在少了 `Message`/`App`/`impl App`,但还留着全部 68 项
App-only 自由函数/类型(这一步不动它们,只处理"它们引用 `Message`/`App`"
和"`impl Workspace` 引用 `Message`"这两类跨文件引用):

1. `workspace.rs` 顶部加 `use crate::app::{App, Message};`(以及任何
   `impl App`/`App` struct 定义里原本用到、但现在只有 `app.rs` 才有的其它
   小类型——按 Step 8 编译报错补全)。
2. 把 68 项留在 `workspace.rs` 的 App-only 符号(见"符号归属清单"里
   `app.rs` 那一节的完整列表)逐个加 `pub(crate)`(类型/结构体本身 +
   如果是自由函数,函数本身)——`app.rs` 的 `impl App`/`Message` 处理器
   要跨文件调用它们。**不含**"符号归属清单"里"留在 `workspace.rs`"那 49
   项——那些本来就不需要跨文件可见。

- [ ] **Step 7: `homespace.rs` 改导入来源**

`homespace.rs` 现有:

```rust
use crate::workspace::{
    App, HoverId, Message, PaneCorner, RailButton, lh, rail_icon_button, relative_time_text,
    zone_pane_border,
};
```

`App`/`Message` 这两个已经在本任务搬到 `app.rs` 了,`HoverId`/
`RailButton`/`PaneCorner`/`rail_icon_button`/`zone_pane_border` 还没搬
(Task 2 才搬,但反正这几个在 Task 1 结束时已经在 `workspace.rs` 标了
`pub(crate)`,`homespace.rs` 从哪个文件导入都能编译通过)——为了 Task 2
时少改一次这行,**这一步直接把它们全部换成从 `crate::app::` 导入**(提前
到位,因为 Task 2 结束后它们确实都在 `app.rs` 里),只有 `lh`/
`relative_time_text` 留在 `crate::workspace::`:

```rust
use crate::app::{App, HoverId, Message, PaneCorner, RailButton, rail_icon_button, zone_pane_border};
use crate::workspace::{lh, relative_time_text};
```

（这一行改了之后,`workspace.rs` 侧 `HoverId`/`RailButton`/`PaneCorner`/
`rail_icon_button`/`zone_pane_border` 这 5 个虽然物理上还没搬,但已经没有
任何代码从 `crate::workspace::` 导入它们了——它们已经不需要因为
`homespace.rs` 的缘故而 `pub(crate)`;是否仍需要 `pub(crate)` 取决于
`app.rs` 侧`impl App`/`Message` 是否直接引用它们,交给 Step 8 编译器判断。)

- [ ] **Step 8: 编译收敛**

```bash
cargo build -p dozer-app 2>&1 | grep -E "^error"
```

反复跑,每次按报错信息修:
- "cannot find type/function X in this scope" → 大概率是 `app.rs` 缺一条
  `use crate::workspace::X;`,或 `workspace.rs` 里的 X 缺 `pub(crate)`。
- "unused import" 警告(不是 error,但顺手清)→ Step 4 复制过来的多余
  `use` 行,删掉。
- 如果报错指向某个具体符号在两个文件里都被引用出问题、怎么调都编不过
  ——大概率是"符号归属清单"漏判的一处,直接把那个符号也剪到 `app.rs`
  (哪怕它这一步"本该"留在 `workspace.rs`),不用纠结,Task 2 反正会把它
  归位。

编译通过后跑:

```bash
cargo test -p dozer-app -- --test-threads=1
cargo clippy -p dozer-app --all-targets
cargo fmt
```

测试数量应与拆分前完全一致(测试还没搬,都留在 `workspace.rs` 原来的
`#[cfg(test)] mod tests` 里,只是它们测的 `App`/`Message` 相关内容现在
是跨文件调用,断言不变)。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/main.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/homespace.rs
git commit -m "refactor(dozer-app): extract App/Message/impl App into app.rs"
```

---

### Task 2: 搬剩余 68 项 App-only 符号到 `app.rs`

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes:Task 1 产出的 `app.rs`(`App`/`Message`/`impl App`)。
- Produces:`app.rs` 内含"符号归属清单"里全部 71 项(Task 1 的 3 大块 +
  这一步的 68 项)。

- [ ] **Step 1: 逐项剪切**

按"符号归属清单 → 归 `app.rs` 的符号"里除 `Message`/`App`/`impl App`
以外的 68 项(壳层小类型 34 项 + 项目页签管理 5 项 + 顶栏渲染 7 项 + 状态点
计算 5 项 + 图标栏/面板区组装 17 项——合计 68),从 `workspace.rs`
逐个剪切,原样粘贴进 `app.rs`(顺序建议保持它们在原文件里的相对先后,
不强求分组分节,与 `homespace.rs`/首页那次拆分"最小改动优先"的取舍一致)。

每剪切一批(建议按上面四个小节分批,每批剪完跑一次 `cargo build -p
dozer-app 2>&1 | grep -E "^error"`,比一次性剪 68 项再排查报错更容易
定位),处理方式同 Task 1 Step 8:缺 `use` 补 `use`,缺 `pub(crate)` 补
`pub(crate)`,分类错了就地挪。

**特别提醒**两处容易漏的地方:
1. `WorkspaceSlot` 挪走后,`workspace.rs` 里 `Workspace` struct 自己的字段
   如果有类型是 `WorkspaceSlot` 的(核对一下——现状 `WorkspaceSlot` 只在
   `App.projects: HashMap<i64, WorkspaceSlot>` 里出现,`Workspace` struct
   本身不含它,预期这条不会触发,但仍需实际编译验证)。
2. `tab_arrow_button`/`tab_divider`/`tab_window` 挪到 `app.rs` 后,
   `workspace.rs` 里的 `preview_pane`(留在 `workspace.rs`)如果调用了
   它们,需要 `use crate::app::{tab_arrow_button, tab_divider,
   tab_window};`。

- [ ] **Step 2: `app.rs` 清理导入**

`app.rs` 顶部 `use crate::workspace::{...}` 那行,现在这 68 项已经不在
`workspace.rs` 了,把它们从这行的导入列表里删掉(留下真正还在
`workspace.rs` 的那 49 项 + `Workspace` 类型本身)。

- [ ] **Step 3: `workspace.rs` 清理多余 `pub(crate)`(可选)**

Task 1 为了过渡给这 68 项加过 `pub(crate)`——它们现在已经不在
`workspace.rs` 了,这条自动失效,不用管。**但** Task 1 里给"留在
`workspace.rs` 的 49 项"没加过 `pub(crate)`(它们本来就不需要),核对一下
这一步搬迁有没有意外牵连到它们(不应该有)。

- [ ] **Step 4: 编译/测试/lint/格式收敛**

同 Task 1 Step 8 的收敛循环,直到:

```bash
cargo build -p dozer-app
cargo test -p dozer-app -- --test-threads=1
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

全部干净通过,**零 unused-import/dead_code 警告**。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs
git commit -m "refactor(dozer-app): move remaining App-only view functions into app.rs"
```

---

### Task 3: 拆分测试模块

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(新增 `#[cfg(test)] mod tests`)
- Modify: `crates/dozer-app/src/workspace.rs`(现有 `#[cfg(test)] mod
  tests` 瘦身)

**Interfaces:** 无新增——纯测试代码搬家,断言不变。

- [ ] **Step 1: 逐个测试分类**

`workspace.rs` 现有 `#[cfg(test)] mod tests`(约 81 个 `#[test]`,合并
首页分支之后从原来的位置起算,`grep -n "#\[cfg(test)\]" crates/dozer-app/
src/workspace.rs` 定位模块起始行)。对每个 `#[test] fn NAME() { .. }`,看
它函数体里主要调用的是"符号归属清单"里 `app.rs` 那 71 项中的哪个,还是
`workspace.rs` 那 49 项中的哪个——跟着它测的那个符号搬家:
- 测的符号在 `app.rs` → 这个测试挪进 `app.rs` 新建的
  `#[cfg(test)] mod tests`。
- 测的符号留在 `workspace.rs` → 测试原地不动。
- 少数测试可能同时调用两侧符号(比如既构造 `ShellState`(App 侧)又调用
  某个 `Workspace` 侧的纯函数)——挑它主要在测的那个符号所在的文件,不必
  纠结,两边都能过就行。

- [ ] **Step 2: 剪切**

按 Step 1 的分类,把该挪的测试函数原样剪切到 `app.rs` 的
`#[cfg(test)] mod tests { use super::*; .. }`(用 `use super::*;` 打头,
跟现有 `workspace.rs`/`homespace.rs` 测试模块的写法一致)。

- [ ] **Step 3: 编译/测试收敛**

```bash
cargo test -p dozer-app -- --test-threads=1 2>&1 | tail -20
```

预期:**测试总数与拆分前完全一致**(只是分布在两个文件的两个测试模块
里),断言全部原样 PASS。如果某个测试因为搬到新文件后编不过(缺
`use`/测试内部依赖了一个仍在另一文件里的私有辅助函数),按报错补
`pub(crate)`/`use`,不改断言内容。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs
git commit -m "test(dozer-app): split workspace.rs test module across app.rs/workspace.rs"
```

---

### Task 4: 全 workspace 收尾验证

**Files:** 无新改动,只跑验证命令。

- [ ] **Step 1: 全量构建/测试/lint/格式**

```bash
cargo build
cargo test
cargo clippy --all-targets
cargo fmt --check
```

预期:全部干净通过,零警告。测试总数与本计划开工前(即
`feature/home-page-4col-layout` 合并进 `main` 之后、这次拆分开工之前)
完全一致——这是纯代码组织重构,不应该有任何测试数量变化。

- [ ] **Step 2: 人工验收**

```bash
cargo run -p dozer-app
```

过一遍基本操作:开项目、切 tab、切左右面板区视图(Files/Web/GitLog/
Todo/Project、Agent/Conversations/Usage/Acceptance)、拖宽面板区、放大/
还原、进出首页(四栏 + 项目列表/Recents 切换 + 全局浏览器)。确认视觉/
交互与拆分前完全一致——人工验收的意义是"确认真的没有改动行为",不是验收
新功能。

- [ ] **Step 3: 确认分支状态**

```bash
git status
git log --oneline main..HEAD
```

确认工作区干净、当前分支只比 `main` 多这次拆分的几个提交,没有意外混入
其它改动。之后按 Global Constraints 提请代码审阅,审阅通过后再合并——不
在这个计划里自动合并。

- [ ] **Step 4: 记录最终文件规模**

```bash
wc -l crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs
```

预期 `app.rs` 落在 4500~5000 行量级(71 项符号 ~4200 行内容 + 拆过去的
测试),`workspace.rs` 落在 2500~3000 行量级(49 项符号 ~2000 行内容 +
留下的测试)——如果哪个文件明显偏离这个量级(比如某个文件还有 6000+ 行),
说明搬迁没搬干净,回去核对"符号归属清单"是否有遗漏项还留在原文件。
