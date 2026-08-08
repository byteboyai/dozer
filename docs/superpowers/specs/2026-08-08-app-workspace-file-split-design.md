# `App`/`Workspace` 文件拆分设计

**状态:已批准(brainstorming 会话,2026-08-08)**

## 背景

`crates/dozer-app/src/workspace.rs` 现有 8797 行,混装了两个概念上完全不同的
层次:`App`(整个程序只有一份的外壳态——daemon 客户端、窗口尺寸、图标栏/面板区
收起-拖宽-放大状态、并行打开的 N 个项目页签)与 `Workspace`(单个项目的全部状态
——终端 tab 集合、文件树、预览/浏览器域、验收与审阅、对话列表)。这次首页四栏化
重构(`docs/superpowers/specs/2026-08-08-home-page-4col-layout-design.md`)
已经把首页专属代码拆进 `homespace.rs`,过程中用户提出:`App`/`Workspace`
这两个概念本身也该物理拆开。

**排期**:首页四栏化的实现已完工并经代码审阅(`feature/home-page-4col-layout`
分支,2 个功能提交 + 1 个修复提交,build/test/clippy/fmt 全干净),但**尚未合并
回 main**。这次拆分排在它之后单独开工,不合并进同一轮改动——理由:两者都要大改
`workspace.rs`,先落地范围更小、已经写好计划的首页工作,能减少这轮拆分要处理的
代码量,也避免两边并行改同一个大文件的合并冲突(教训见本仓库过往经验:两个 agent
同时在同一分支改同一个大文件会让审阅方难以区分改动归属)。

## 现状勘查(brainstorming 会话实测)

- 全文件 8797 行。类型定义/`Message` 枚举等前置内容约 1473 行(1-1473);
  `impl Workspace` 843 行(1474-2317,40 个方法);`impl App` 1843 行
  (2318-4161);`impl` 块之外的自由函数(视图渲染为主)约 3121 行
  (4161-7282);`#[cfg(test)] mod tests` 约 1515 行(7282-8797)。
- **`impl Workspace` 的 40 个方法里,零处真实依赖 `App` 类型**(唯一出现
  `App` 字样的地方是文档注释,不是代码)。这不是巧合——`ShellIo`(`client`/
  `handle`/`proxy`/`cols`/`rows` 的 `Clone` 值类型打包)的存在就是为了让
  `Workspace` 方法拿到 IO 句柄时**按值**收,不反向借用 `App`(文档注释原文:
  `App::active_workspace_mut()` 交出的 `&mut Workspace` 本身已经是从 `&mut
  self` 借出去的,此时再让 `Workspace` 方法借 `&self.client` 就是同时可变+
  不可变借 `self`,过不了借用检查器)。换句话说,现状代码已经主动避免了
  `Workspace → App` 的依赖,只是两者仍physically 挤在同一个文件里。
- 自由函数里,只有 **6 个**同时以 `app: &App`/`ws: &Workspace` 为参数:
  `left_panel_area`/`right_panel_area`/`maximize_overlay`/`terminal_pane`/
  `tab_bar`/`active_tab_view`。它们都是"外壳把某个项目的内容组装进面板区"
  这一类函数——语义上属于外壳(`App` 决定布局/收起/放大态,`Workspace` 提供
  内容),不属于 `Workspace` 自己。
- `Message` 枚举(890 行起)是 `App::update` 的唯一分发入口,`App` 是它的
  真正宿主;但 `Workspace` 的异步方法(`spawn_*` 系列)也要构造 `Message`
  变体经 `EventLoopProxy<Message>` 送回 UI 线程——这意味着拆开后两个文件会
  **互相 `use` 对方的类型**(`workspace.rs` 需要 `crate::app::Message`,
  `app.rs` 需要 `crate::workspace::Workspace`)。这在 Rust 里完全合法,不是
  "循环依赖"问题——模块间互相引用对方的 `pub`/`pub(crate)` 项不受编译顺序
  限制,与脚本语言的 import 环是两回事,不需要为此设计任何"打破环"的中间层。
- `LeftView`/`RightView`/`Divider`/`PaneCorner`/`HoverId`/`RailButton`/
  `AppPage`/`ZoneSide`/`MaximizedPane`/`ShellState`/`ShellLayout` 这批"壳层
  状态"小类型,实测 `impl Workspace` 与"只依赖 `Workspace`"的自由函数里完全
  不用它们——纯 `App` 概念,拆分时整体归 `app.rs`。

## 目标 / 非目标

**目标**:

1. 新建 `crates/dozer-app/src/app.rs`(`main.rs` 加 `mod app;`),移入:
   - `App` struct + `impl App`(含 `Message` 枚举——`App::update` 是它唯一的
     分发入口,归属明确)。
   - 只依赖 `App`(不依赖 `Workspace`)的自由函数,例如顶栏
     (`top_bar`/`dozer_home_tab`/`project_tabs_row`/`project_tab_item`
     等)、左右图标栏(`left_icon_rail`/`right_icon_rail`/
     `rail_icon_button`)等。
   - 6 个"既要 `App` 又要 `Workspace`"的组装函数:`left_panel_area`/
     `right_panel_area`/`maximize_overlay`/`terminal_pane`/`tab_bar`/
     `active_tab_view`(通过 `use crate::workspace::Workspace;` 单向引入
     `Workspace` 类型,不构成环)。
   - 壳层状态小类型(`LeftView`/`RightView`/`Divider`/`PaneCorner`/
     `HoverId`/`RailButton`/`AppPage`/`ZoneSide`/`MaximizedPane`/
     `ShellState`/`ShellLayout`)——**不重新分组**,按它们在原文件里的相对
     先后顺序搬,原样堆在 `app.rs` 顶部(已与用户确认:最小改动优先,降低
     搬迁 diff 的可读性噪声,反正都在同一个文件里,没有再分组的必要)。
   - 对应的 `#[cfg(test)] mod tests` 子集(测哪个函数,测试就跟哪个函数走)。
2. `workspace.rs`(保留原文件名,内容大幅缩减)只剩:`Workspace` struct +
   `ShellIo` + `impl Workspace`(40 个方法原样不动)+ 只依赖 `Workspace`
   的自由函数(项目树/预览/终端等按项目分的渲染逻辑,如 `preview_pane`/
   `worktree_strip` 等)+ 对应测试。**不 `use crate::app::` 除 `Message`
   以外的任何东西**——`Message` 是唯一允许的例外(`Workspace` 的
   `spawn_*` 异步方法要用它构造回传消息)。
3. 纯代码搬家 + 可见性调整(哪些类型/字段/函数从"仅本文件可见"升级到
   `pub(crate)`,视搬迁后跨文件引用需要而定,做法与 `homespace.rs` 那次
   拆分完全一致)。**不改任何行为**——UI 观感、消息流转、状态语义、既有
   测试断言全部原样保留。
4. 预期两个文件各自落在 4000~4500 行量级(比现在单文件 8797 行好控制),
   目录结构仍是扁平 `src/`(不引入 `app/`/`workspace/` 子目录——已与用户
   确认这次不做更细粒度的多文件拆分,YAGNI)。

**非目标**:

- 不改变任何运行时行为——这是纯代码组织重构,不是功能变更。
- 不引入 `app/`/`workspace/` 子目录形态的更细粒度拆分。
- 不顺带重构 `impl Workspace`/`impl App` 内部的任何方法实现、不合并/拆分
  任何既有方法、不改任何字段的语义。
- 不改 `Message` 枚举任何变体的名字/载荷类型——只搬它的物理位置。
- 不影响 `homespace.rs`/`extensions/*`——它们已经是独立文件,`use
  crate::workspace::{...}` 的地方按需要改成 `use crate::app::{...}`,仅此
  而已,不重新设计它们的接口。
- 不在这轮改动里合并 `feature/home-page-4col-layout` 分支——两者分开走,
  这轮拆分基于（已审阅但未合并的)首页四栏化改动之后的 `main` 状态开工,
  具体排期由用户决定何时把首页分支合并回 main。

## 架构与数据流

### 1. 依赖方向(最终态)

```
main.rs
  ├── mod app;        // App + Message + impl App + App-only 视图 + 6 个混合组装函数 + 壳层小类型
  │     use crate::workspace::Workspace;   // 单向:app 依赖 workspace 的类型
  │     use crate::extensions::*;          // 不变
  │     use crate::homespace;              // 不变(home_page(app: &App) 调用点仍在 app.rs 的 App::view 里)
  │
  └── mod workspace;  // Workspace + ShellIo + impl Workspace + Workspace-only 视图
        use crate::app::Message;           // 单向:workspace 只借用 Message 这一个类型
```

`homespace.rs`/`extensions/*.rs` 不改依赖方向,只是它们现有的
`use crate::workspace::{App, ...}` 这类导入,视具体符号新家改成
`use crate::app::{App, ...}` 或保持 `use crate::workspace::{Workspace,
...}`——逐符号核对,不是整体批量改。

### 2. 拆分边界判定标准

每个符号(类型/字段/函数)按下面这条规则归属,与"壳层状态小类型"那批的
判断依据一致:

- **只出现在 `impl Workspace` 或"只依赖 `Workspace`"的自由函数里** → 归
  `workspace.rs`。
- **只出现在 `impl App` 或"只依赖 `App`"的自由函数里,或是纯壳层状态**
  (不被任何 `Workspace` 侧代码引用)→ 归 `app.rs`。
- **`Message` 枚举本身** → 归 `app.rs`(`App::update` 的分发入口)。
- **6 个"既要 `App` 又要 `Workspace`"的组装函数** → 归 `app.rs`(它们是
  外壳把内容摆进面板区,产品语义上属于外壳,不属于单个项目)。
- **第五类,写计划时才会具体碰到**:一批签名里不带 `app`/`ws` 参数、纯靠
  值计算的工具函数(如 `zone_pane_border`/`PaneCorner`/`relative_time_text`
  /`lh`),现状是 `workspace.rs` 一个文件内自然共享,拆分后会被 `app.rs`/
  `workspace.rs`/`homespace.rs` 至少两侧同时引用。这批不强求"哪边用得多归
  哪边"这类精确规则,写计划时按它更贴近的语义域拣一边落地(壳层几何类的
  如 `zone_pane_border`/`PaneCorner` 落 `app.rs`,纯字符串/时间格式化类的
  如 `relative_time_text` 落哪边均可),标 `pub(crate)` 供另一侧引用即可
  ——这类函数本来就不持有状态、不构成拆分边界的实质困难,不值得为了"选边"
  纠结。

### 3. 可见性调整

搬迁后凡是需要跨文件引用的符号,按 `homespace.rs` 那次拆分定下的口径处理:
默认最小可见性 `pub(crate)`(不用 `pub`,除非该符号本来就需要给
`extensions/*`/`homespace.rs` 之外的更大范围用)。写实现计划时逐个核对现状
（哪些已经是 `pub`/`pub(crate)`,哪些需要新加),不预先假设。

### 4. 测试搬迁

`#[cfg(test)] mod tests` 按"测的函数归哪个文件,测试就跟去哪个文件"拆成两份,
不共用一个测试模块——与 `homespace.rs` 拆分时 `load_home_recents_*` 三个测试
的处理方式一致。

## 错误处理

不适用——纯代码组织重构,没有新的运行时错误路径。

## 测试策略

- **零行为变更验证**:拆分完成后,`cargo test`(全 workspace)的测试集合
  (数量、名字、断言)应该与拆分前完全一致,只是物理分布在两个文件里而非
  一个——不新增、不删除、不修改任何一条既有测试的断言内容。
- 每个任务结束都要 `cargo build && cargo test && cargo clippy --all-targets
  && cargo fmt` 干净通过(全 workspace,不只 `dozer-app`——虽然预期只有
  `dozer-app` 会变,但按仓库既有惯例每次都跑全量)。
- 人工验收:`cargo run -p dozer-app` 跑起来,过一遍基本操作(开项目、切
  tab、切左右面板区视图、拖宽、放大/还原、进出首页)确认视觉/交互与拆分前
  完全一致——这是纯重构,人工验收的意义是"确认真的没有改动行为",不是验
  收新功能。

## 依赖变更

无新增依赖。
