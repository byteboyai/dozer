# Dozer 主界面外壳重构——图标栏 + Pane 放大 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `dozer-app` 现有的"项目|预览|终端|AI"四栏固定同时显示布局，替换成"左图标栏+左面板区+右面板区+右图标栏"的模式切换布局，并给每个内容 pane 加放大到全窗覆盖的能力。

**Architecture:** 左/右各一条固定图标栏（图标→视图的映射本轮固定，不做拖拽重排），每侧当前视图决定该侧面板区渲染什么——文件列表(项目树+文件预览)/Web(单面板) 在左，Agent(Agent列表+终端)/对话(对话列表+对话审阅) 在右。绝大多数现有 pane 渲染函数（`project_pane`/`preview_pane`/`terminal_pane`）原样保留内部逻辑，只是外层宽度从"读 `ws.layout.某字段`"改成"接收调用方传入的 `width: f32` 参数"——因为宽度不再是四栏各自独立的字段，而是由左右面板区总宽 × 内部分割比例算出来的。`ai_pane` 被拆成 `agent_list_pane`/`conversation_list_pane` 两个更小的函数（原来内部靠 `AiView` pill 切换的两块内容，现在由图标栏直接决定显示哪个，不再需要 pill）。放大是层叠(overlay)而非布局重排——`view()` 的 `stack!` 链新增一层，背后内容变暗保留渲染。

**Tech Stack:** Rust, iced 0.14（复用现有 `MouseArea`/`divider_bar`/`stack!` 手算定位风格），复用上一会话已建好的 `icons::view`/`IconKind` 图标模块。

## Global Constraints

- 颜色只取自 `crates/dozer-app/src/theme.rs` 现有 14 个常量，禁止新增硬编码色值。
- 每 task 收尾 `cargo build -p dozer-app`、`cargo test -p dozer-app` 绿、`cargo clippy -p dozer-app --all-targets` clean、`cargo fmt -p dozer-app -- --check` 干净。
- `dozer-app` 是纯 `[[bin]]` crate（无 `[lib]` target），`pub` 不豁免 `dead_code` lint——本计划新模块/新枚举变体在未被消费前需要 `#[allow(dead_code)]`（跟随 `theme.rs`/上一会话 `icons.rs` 的模块级 allow 先例，不做逐条摘除的繁琐流程）。
- **本计划范围仅 §2(整体外壳架构)+ §4(pane 放大)**，不含 §3(项目页签/并行多项目)——那部分已在设计文档里标记为独立后续设计任务，不在这里实现。顶栏 `top_bar` 函数本计划完全不touch。
- **验收(Acceptance)维持现状不变**——它目前是 `preview_pane` 内部与 File/Web 共享同一个 tab 系统的一种 `TabKind`，本计划不改这一点（即便这与 Figma S2 的"全屏接管"设计不完全一致——那是预先存在的差距，不在本计划范围内解决，见设计文档 §2.1）。
- 设计依据：`docs/superpowers/specs/2026-07-29-dozer-shell-icon-rail-design.md` §2、§4。

## 已知的一处简化，写在前面（避免被当成遗漏）

设计文档把"文件列表"和"Web"列为两个独立图标，字面读法可能暗示两者显示不同的、各自过滤过的标签页集合。但现状 `PreviewPane`（`preview.rs`）把 File/Web/Acceptance 三种 tab 混在同一个 `tabs: Vec<PreviewTab>` + 同一个全局 `active` 索引里，没有按种类分组的概念；把它们拆成独立分组需要新增"按组各自记住激活项"的状态，是比"外壳重排"大一截的数据模型改动，设计文档没有明确要求到这个精度。

本计划采用的简化：**"文件列表"和"Web"两个图标背后是同一个 `preview_pane` 渲染函数、同一份 tab 数据——区别只是"文件列表"图标额外把项目树摆在它左边，"Web"图标单独占满左面板区宽度（不摆项目树）**。也就是说 Web 图标目前主要起到"腾出项目树的位置，给内容更多宽度"的作用，不是"只看 Web 类标签页"的过滤视图。如果这与你的预期不符，请在本计划执行前提出，改起来是这个计划里风险最高的一处返工点。

---

### Task 1: 新数据模型——枚举/结构体 + `Workspace` 字段 + 持久化换型

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（新增 `LeftView`/`RightView`/`MaximizedPane` 枚举；`PanelLayout` 换成 `ShellLayout`；`Divider` 换成新的三个变体；`Workspace` 新增字段）
- Modify: `crates/dozer-app/src/layout.rs`（`load`/`save`/测试全部换成 `ShellLayout`）

**Interfaces:**
- Produces:
  - `pub enum LeftView { Files, Web }`（`#[derive(Debug, Clone, Copy, PartialEq)]`）
  - `pub enum RightView { Agent, Conversations }`（同上）
  - `pub enum MaximizedPane { Left, Right }`（同上）
  - `pub struct ShellLayout { pub left_width: f32, pub files_split: f32, pub agent_split: f32, pub conversations_split: f32 }`（`#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]`，`impl Default`）
  - `pub enum Divider { LeftRight, LeftPairSplit, RightPairSplit }`（替换原 `ProjectPreview`/`PreviewTerminal`/`TerminalAi` 三个变体）
  - `pub const ICON_RAIL_WIDTH: f32 = 48.0;`
- Consumes: 无（本任务不删除任何仍被引用的旧类型——`PanelLayout`/旧 `Divider` 变体/`apply_column_drag`/`preview_content_bounds` 等仍原样存在，本任务只是新增，不碰它们，避免破坏编译）。

本任务只做新增，不删旧的、不改 `view()`。新增的类型在本任务结束时没有任何调用方，会被 `dead_code` lint 标记——按 `theme.rs`/上一会话 `icons.rs` 的先例，加模块级或就近的 `#[allow(dead_code)]`。

- [ ] **Step 1: 写失败测试（`ShellLayout` 持久化往返 + 各枚举 `Default`/相等性）**

在 `crates/dozer-app/src/layout.rs` 的 `#[cfg(test)] mod tests` 里追加（这些测试要求 `layout.rs` 的 `load_from`/`save_to` 已经改成吃 `ShellLayout`，本步骤先写测试确认失败，Step 3 再改实现）：

```rust
    #[test]
    fn shell_layout_default_has_sane_values() {
        let l = ShellLayout::default();
        assert!(l.left_width > 0.0);
        assert!((0.0..=1.0).contains(&l.files_split));
        assert!((0.0..=1.0).contains(&l.agent_split));
        assert!((0.0..=1.0).contains(&l.conversations_split));
    }

    #[test]
    fn shell_layout_save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("shell_layout.json");
        let layout = ShellLayout {
            left_width: 500.0,
            files_split: 0.4,
            agent_split: 0.35,
            conversations_split: 0.45,
        };
        save_to(&path, &layout).unwrap();
        assert_eq!(load_from(&path), layout);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer layout::tests::shell_layout`
Expected: FAIL（编译错误，`ShellLayout` 未定义）。

- [ ] **Step 3: 在 `workspace.rs` 新增枚举/结构体**

在 `crates/dozer-app/src/workspace.rs` 里，紧邻现有 `pub struct PanelLayout { ... }`（第 57-63 行）**之后**（不删除 `PanelLayout` 本身，本任务两者共存）新增：

```rust
/// 左侧面板区当前显示哪个视图：文件列表(项目树+文件预览配对) / Web(单面板)。
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)] // 本任务先建好类型，Task 3 起接入图标栏/面板渲染
pub enum LeftView {
    Files,
    Web,
}

/// 右侧面板区当前显示哪个视图：Agent(Agent列表+终端配对) / 对话(对话列表+对话审阅配对)。
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum RightView {
    Agent,
    Conversations,
}

/// 当前放大态：放大的是左面板区的内容子面板，还是右面板区的。`None` = 未放大。
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum MaximizedPane {
    Left,
    Right,
}

/// 图标栏+左右面板区的宽度/分割状态。取代 `PanelLayout`——不再有"项目栏/AI栏
/// 固定宽+预览终端共享比例"这套四栏几何，改成"左面板区总宽(可拖) + 三个
/// 配对视图各自独立记住的内部列表:内容分割比例"。右面板区总宽不持久化，
/// 恒为剩余空间(`Length::Fill`)——只有一条 LeftRight 分隔线，不需要像旧
/// 模型那样两个固定宽度各自夹一条。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ShellLayout {
    pub left_width: f32,
    /// 文件列表配对:项目树占左面板区宽度的比例，文件预览拿剩下的。
    pub files_split: f32,
    /// Agent配对:Agent列表占右面板区宽度的比例，终端拿剩下的。
    pub agent_split: f32,
    /// 对话配对:对话列表占右面板区宽度的比例，对话审阅拿剩下的。
    pub conversations_split: f32,
}

impl Default for ShellLayout {
    fn default() -> Self {
        Self {
            left_width: 640.0,
            files_split: 0.35,
            agent_split: 0.4,
            conversations_split: 0.4,
        }
    }
}
```

紧邻现有 `pub enum Divider { ProjectPreview, PreviewTerminal, TerminalAi }`（第 77-81 行）**之后**（同样不删旧的）新增：

```rust
/// 新外壳的三条可拖拽分隔线：左右面板区之间、左侧配对视图内部、右侧配对
/// 视图内部。取代 `Divider` 原三个变体(见 Task 3 的整体切换)。
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum ShellDivider {
    LeftRight,
    LeftPairSplit,
    RightPairSplit,
}

/// 图标栏固定宽度(逻辑像素)，左右各一条。
pub const ICON_RAIL_WIDTH: f32 = 48.0;
```

（注意：本任务新分隔线枚举暂命名 `ShellDivider`，不是直接叫 `Divider`——因为 `Divider` 这个名字现在被旧枚举占着，Task 3 做整体切换时会把旧 `Divider` 删掉、把 `ShellDivider` 重命名回 `Divider`。这是故意的中间状态，不是命名不一致的 bug。）

- [ ] **Step 4: 改 `layout.rs` 的 `load_from`/`save_to`/`load`/`save` 签名**

把 `crates/dozer-app/src/layout.rs` 顶部的：

```rust
use crate::workspace::PanelLayout;
```

改为：

```rust
use crate::workspace::ShellLayout;
```

把文件里 4 个函数签名的 `PanelLayout` 全部替换成 `ShellLayout`（`pub fn load() -> PanelLayout` → `-> ShellLayout`；`pub fn save(layout: &PanelLayout)` → `&ShellLayout`；`fn load_from(path: &Path) -> PanelLayout` → `-> ShellLayout`；`fn save_to(path: &Path, layout: &PanelLayout)` → `&ShellLayout`）。函数体不用改，纯泛型走 serde 的读写逻辑对两个类型是一样的。

已有的 3 个测试（`load_from_missing_file_returns_default`/`load_from_corrupt_file_returns_default`/`save_then_load_round_trips`）里的 `PanelLayout` 字面量构造（`project_col_width`/`ai_col_width`/`preview_ratio` 三个字段）也要同步换成 `ShellLayout` 的字段（`left_width`/`files_split`/`agent_split`/`conversations_split`）：

```rust
    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("shell_layout.json");
        let layout = ShellLayout {
            left_width: 500.0,
            files_split: 0.4,
            agent_split: 0.35,
            conversations_split: 0.45,
        };
        save_to(&path, &layout).unwrap();
        assert_eq!(load_from(&path), layout);
    }
```

（这一步会让 Step 1 新加的 `shell_layout_save_then_load_round_trips` 与这条已有测试重复——保留 Step 1 那条，删掉这条改名后的重复测试，避免两条断言完全一样的测试同时存在。）

**这一步会让 `workspace.rs` 里 `layout: layout::load()`（两处构造函数）编译失败**（`layout::load()` 现在返回 `ShellLayout`，但 `Workspace.layout` 字段类型还是 `PanelLayout`）——这是预期的、本步骤刻意暂不修的中间态。继续到 Step 5。

- [ ] **Step 5: `Workspace` 新增字段（先不删旧 `layout: PanelLayout` 字段，改名共存）**

把 `crates/dozer-app/src/workspace.rs` 里 `Workspace` 结构体的：

```rust
    layout: PanelLayout,
```

改为（新旧两个字段共存，本任务结束时旧的 `PanelLayout` 字段真正变成完全未读的死字段——这是本任务唯一一处"旧字段变死"的情况，Task 3 会删掉它）：

```rust
    #[allow(dead_code)] // Task 3 整体切换后删除，本任务先建好新字段
    old_layout: PanelLayout,
    shell_layout: ShellLayout,
    left_view: LeftView,
    right_view: RightView,
    left_collapsed: bool,
    right_collapsed: bool,
    maximized: Option<MaximizedPane>,
```

两处构造函数（`bootstrap`/`with_daemon_error`）里找到 `layout: layout::load(),`（`bootstrap`）和 `layout: layout::load(),`（`with_daemon_error`，若两处写法不同以实际读到的为准），改成：

```rust
            old_layout: PanelLayout::default(),
            shell_layout: layout::load(),
            left_view: LeftView::Files,
            right_view: RightView::Agent,
            left_collapsed: false,
            right_collapsed: false,
            maximized: None,
```

（`old_layout` 暂时用 `PanelLayout::default()` 占位，不再从磁盘读——旧的 `layout::load()` 现在返回 `ShellLayout` 类型，语义上已经不该再喂给 `old_layout: PanelLayout`。这个字段本任务结束时纯粹是编译占位，Task 3 会整个删掉。）

- [ ] **Step 6: 编译 + 测试 + clippy + fmt**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 编译通过。如果报错，最可能是某处仍直接构造/读取旧 `Workspace.layout`字段名（现在叫 `old_layout`）或某个已有函数签名仍写着 `PanelLayout` 而调用方传了 `ShellLayout`——按报错逐一改字段/类型名，不要改动本步骤未提到的逻辑。

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/layout.rs
git commit -m "feat(shell): 新外壳数据模型(LeftView/RightView/ShellLayout/ShellDivider,与旧四栏模型暂共存)"
```

---

### Task 2: Vendor 4 个新 Lucide 图标 + `IconKind` 新变体

**Files:**
- Create: `crates/dozer-app/assets/icons/globe.svg`、`bot.svg`、`message-square.svg`、`maximize-2.svg`
- Modify: `crates/dozer-app/src/icons.rs`（`IconKind` 新增 4 个变体）

**Interfaces:**
- Consumes: 无。
- Produces: `IconKind::{Globe, Bot, MessageSquare, Maximize}`，`icons::view(kind, size, color)` 对这 4 个新变体可用（复用已有 `view()` 函数，无需改签名）。

- [ ] **Step 1: 下载 4 个 Lucide 图标**

```bash
mkdir -p crates/dozer-app/assets/icons
cd crates/dozer-app/assets/icons
for name in globe bot message-square maximize-2; do
  curl -fsSL -o "${name}.svg" \
    "https://raw.githubusercontent.com/lucide-icons/lucide/main/icons/${name}.svg"
done
cd -
```

Run: `for f in crates/dozer-app/assets/icons/{globe,bot,message-square,maximize-2}.svg; do head -c 40 "$f"; echo " <- $f"; done`
Expected: 4 行输出全部以 `<svg` 开头（可能前面有 `<?xml ...?>`），不是 HTML。若某个 slug 404（`curl -f` 会让该行非零退出），去 `https://lucide.dev/icons` 搜索对应概念确认当前正确的图标名，替换 slug 重跑——这是应对第三方图标库命名可能随版本漂移的操作性预案，上一会话的图标计划也遇到过一次(`file-json`→`braces`)，处理方式相同。

- [ ] **Step 2: `IconKind` 新增变体**

在 `crates/dozer-app/src/icons.rs` 的 `pub enum IconKind { ... }` 里，紧邻现有最后一个变体 `Rename,` 之后新增：

```rust
    Globe,
    Bot,
    MessageSquare,
    Maximize,
```

在 `impl IconKind { fn bytes(self) -> &'static [u8] { match self { ... } } }` 的 `match` 里，紧邻 `IconKind::Rename => include_bytes!("../assets/icons/pen-line.svg"),` 之后新增：

```rust
            IconKind::Globe => include_bytes!("../assets/icons/globe.svg"),
            IconKind::Bot => include_bytes!("../assets/icons/bot.svg"),
            IconKind::MessageSquare => include_bytes!("../assets/icons/message-square.svg"),
            IconKind::Maximize => include_bytes!("../assets/icons/maximize-2.svg"),
```

- [ ] **Step 3: 编译 + 测试 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。（新变体暂无调用方——`icons.rs` 模块已有 `#![allow(dead_code)]`，见上一会话先例，不需要逐个再加。）

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/assets/icons crates/dozer-app/src/icons.rs
git commit -m "feat(shell): vendor globe/bot/message-square/maximize-2 图标 + IconKind 新变体"
```

---

### Task 3: 外壳整体切换——面板函数改宽度参数 + 几何重算 + 图标栏 + `view()` 重装

这是本计划最大的一个任务，**不可再拆**：下面列出的每一处改动都因为 Rust 的强类型检查而必须同时生效——改了 `preview_pane` 的函数签名，`view()` 里那一处调用点当场就编译不过，直到 `view()` 本身也改完；main.rs 的 4 个几何函数调用点同理。拆成"独立可编译"的小任务在这里是假的粒度，会制造中间态骗自己。按下面的 Step 顺序做，中途不要求每个 Step 单独编译（只有整个 Task 完成后才编译），但每个 Step 仍然是一次独立、可核对的改动。

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（删旧 `Divider`/`PanelLayout`/`apply_column_drag`/`preview_content_bounds`/`is_in_preview_column`/`terminal_pane_pixel_size`/`old_layout` 字段；`ShellDivider`→`Divider`；`project_pane`/`preview_pane`/`terminal_pane` 改签名；`ai_pane` 拆成两个函数；新增 `review_content_pane`/`left_icon_rail`/`right_icon_rail`/`left_panel_area`/`right_panel_area`/新几何函数；`view()` 重写；`ime_cursor_area` 改用新几何；`Message` 新增 `LeftIconSelect`/`RightIconSelect`；`update()` 新增分支并改 `ColumnDrag`/`ColumnDragEnd` 分支；`AiView`/`Message::AiViewSwitch`/`ws.ai_view` 删除；`TabKind::Review`/`open_review`/`review_active`/`ConversationOpen` 里的 `open_review()` 调用删除）
- Modify: `crates/dozer-app/src/preview.rs`（删 `TabKind::Review`/`open_review`/`review_active`，相应 match 分支收窄）
- Modify: `crates/dozer-app/src/main.rs`（4 处几何函数调用点、divider hit-testing、焦点路由，全部适配新签名/新语义）

**Interfaces:**
- Consumes: Task 1 的 `LeftView`/`RightView`/`MaximizedPane`/`ShellLayout`/`ShellDivider`（本任务重命名为 `Divider`）/`ICON_RAIL_WIDTH`；Task 2 的 `IconKind::{Globe,Bot,MessageSquare}`（`Maximize` 留给 Task 5）。
- Produces:
  - `pub struct ShellState { pub layout: ShellLayout, pub left_view: LeftView, pub left_collapsed: bool, pub right_view: RightView, pub right_collapsed: bool }`（`#[derive(Debug, Clone, Copy, PartialEq)]`）
  - `Workspace::shell_state(&self) -> ShellState`（取代旧 `Workspace::layout(&self) -> PanelLayout`）
  - `pub fn preview_content_bounds(window_width: f32, window_height: f32, state: &ShellState) -> (f32, f32, f32, f32)`（签名变了：第三参从 `&PanelLayout` 变成 `&ShellState`）
  - `pub fn is_in_preview_column(x: f32, window_width: f32, state: &ShellState) -> bool`（同样改签名）
  - `pub fn terminal_pane_pixel_size(window_width: f32, window_height: f32, state: &ShellState) -> (f32, f32)`（同样改签名）
  - `fn left_zone_width(window_width: f32, state: &ShellState) -> f32`、`fn right_zone_width(window_width: f32, state: &ShellState) -> f32`（新的私有几何辅助函数，上面 3 个 pub 函数与 `apply_column_drag` 内部都靠它们算"左/右面板区当前实际宽度"，避免 4 处各写各的重复公式——这是本任务顺便修的一个既有小坏味道，Explore 阶段已确认旧代码 4 处重复同一条 `fill_width` 公式）
  - Task 4/5 会消费：`Workspace::left_view()`/`right_view()`/`left_collapsed()`/`right_collapsed()`/`maximized()` 只读 getter（渐进只读访问器，同 `dragging_divider()` 先例）

- [ ] **Step 1: 删除旧几何常量与函数，新增几何辅助函数**

删除 `crates/dozer-app/src/workspace.rs` 里的：
- `const MIN_PROJECT_COL_WIDTH: f32 = 180.0;`
- `const MIN_AI_COL_WIDTH: f32 = 200.0;`
- `const MIN_FILL_WIDTH: f32 = 480.0;`
- `const MIN_PREVIEW_RATIO: f32 = 0.15;`
- `const MAX_PREVIEW_RATIO: f32 = 0.85;`
- 整个 `fn apply_column_drag(layout: PanelLayout, divider: Divider, window_width: f32, logical_x: f32) -> PanelLayout { ... }` 函数体（第 126-163 行一带）
- 整个 `pub fn preview_content_bounds(...) -> (f32,f32,f32,f32) { ... }`（第 193-206 行）
- 整个 `pub fn is_in_preview_column(...) -> bool { ... }`（第 210-217 行）
- 整个 `pub fn terminal_pane_pixel_size(...) -> (f32,f32) { ... }`（第 222-234 行）
- `pub struct PanelLayout { ... }` 及其 `impl Default`（第 57-73 行）
- `pub enum Divider { ProjectPreview, PreviewTerminal, TerminalAi }`（第 77-81 行）

（`DIVIDER_WIDTH`/`TOP_BAR_HEIGHT`/`STATUS_BAR_HEIGHT`/`CONTEXT_MENU_WIDTH`/`CONTEXT_MENU_HEIGHT`/`CHROME_WIDTH_PX`/`CHROME_HEIGHT_PX`/`PREVIEW_CHROME_TOP_PX` 都保留不动，新几何函数还要用。）

把 Task 1 新增的 `ShellDivider` 重命名为 `Divider`（即删掉重复，直接把它当成正式的 `Divider` 定义——把类型定义里的 `pub enum ShellDivider` 改成 `pub enum Divider`），去掉它头上的 `#[allow(dead_code)]`（本任务马上就有真实调用方）。

新增新的最小宽度常量与几何辅助函数，放在原来 `apply_column_drag` 所在位置附近：

```rust
const MIN_ZONE_WIDTH: f32 = 320.0;
const MIN_SPLIT_RATIO: f32 = 0.2;
const MAX_SPLIT_RATIO: f32 = 0.8;

/// 主界面当前几何状态的只读快照(main.rs 拖拽追踪/离屏几何计算用途,
/// `Copy` 类型直接按值传递)。取代旧 `PanelLayout` 单独传递的做法——
/// 新几何公式(webview bounds/焦点路由/IME 光标)都依赖"当前是哪个视图、
/// 是否收起"，不能只靠宽高数字算，所以把这些也打包进来。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShellState {
    pub layout: ShellLayout,
    pub left_view: LeftView,
    pub left_collapsed: bool,
    pub right_view: RightView,
    pub right_collapsed: bool,
}

/// 左面板区当前实际宽度(逻辑像素)；收起时为 0。
fn left_zone_width(state: &ShellState) -> f32 {
    if state.left_collapsed {
        0.0
    } else {
        state.layout.left_width
    }
}

/// 右面板区当前实际宽度(逻辑像素)；收起时为 0；否则是"总宽减两条图标栏、
/// 减左面板区、减(两侧都可见时的)一条分隔线"的剩余空间——右面板区不像
/// 左面板区那样有独立持久化宽度，恒为 Fill。
fn right_zone_width(window_width: f32, state: &ShellState) -> f32 {
    if state.right_collapsed {
        return 0.0;
    }
    let left_w = left_zone_width(state);
    let both_visible = !state.left_collapsed && !state.right_collapsed;
    let divider_w = if both_visible { DIVIDER_WIDTH } else { 0.0 };
    (window_width - 2.0 * ICON_RAIL_WIDTH - left_w - divider_w).max(0.0)
}

/// 拖拽某条分隔线到窗口逻辑 x 坐标 `logical_x` 后的新 `ShellLayout`。
/// `LeftPairSplit`/`RightPairSplit` 写哪个 split 字段取决于当前那一侧的
/// 视图选择(比如右侧当前是"对话"就写 `conversations_split`，不是
/// `agent_split`)——这条信息 `ShellLayout` 自己没有，靠 `ShellState` 带过来。
fn apply_column_drag(
    state: ShellState,
    divider: Divider,
    window_width: f32,
    logical_x: f32,
) -> ShellLayout {
    match divider {
        Divider::LeftRight => {
            let both_visible = !state.left_collapsed && !state.right_collapsed;
            let divider_w = if both_visible { DIVIDER_WIDTH } else { 0.0 };
            let upper = (window_width - 2.0 * ICON_RAIL_WIDTH - divider_w - MIN_ZONE_WIDTH)
                .max(MIN_ZONE_WIDTH);
            let new_left = (logical_x - ICON_RAIL_WIDTH).clamp(MIN_ZONE_WIDTH, upper);
            ShellLayout {
                left_width: new_left,
                ..state.layout
            }
        }
        Divider::LeftPairSplit => {
            let left_w = left_zone_width(&state);
            if left_w <= 0.0 {
                return state.layout;
            }
            let ratio =
                ((logical_x - ICON_RAIL_WIDTH) / left_w).clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
            ShellLayout {
                files_split: ratio,
                ..state.layout
            }
        }
        Divider::RightPairSplit => {
            let right_w = right_zone_width(window_width, &state);
            if right_w <= 0.0 {
                return state.layout;
            }
            let right_x0 = window_width - ICON_RAIL_WIDTH - right_w;
            let ratio = ((logical_x - right_x0) / right_w).clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
            match state.right_view {
                RightView::Agent => ShellLayout {
                    agent_split: ratio,
                    ..state.layout
                },
                RightView::Conversations => ShellLayout {
                    conversations_split: ratio,
                    ..state.layout
                },
            }
        }
    }
}

/// 窗口逻辑尺寸 → 左侧文件/Web 预览内容区矩形(逻辑像素 x/y/w/h)，供
/// main.rs 摆放 wry webview 用。左侧收起、或当前左视图不是 Files/Web 时
/// (理论上左视图恒为其中之一，这个分支是防御性兜底)返回零尺寸矩形。
pub fn preview_content_bounds(
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32, f32, f32) {
    if state.left_collapsed {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let left_w = left_zone_width(state);
    let y = TOP_BAR_HEIGHT + PREVIEW_CHROME_TOP_PX;
    let h = (window_height - y - 8.0).max(0.0);
    match state.left_view {
        LeftView::Web => {
            let x = ICON_RAIL_WIDTH + 8.0;
            let w = (left_w - 16.0).max(0.0);
            (x, y, w, h)
        }
        LeftView::Files => {
            let list_w = left_w * state.layout.files_split;
            let x = ICON_RAIL_WIDTH + list_w + DIVIDER_WIDTH + 8.0;
            let content_w = left_w - list_w - DIVIDER_WIDTH;
            let w = (content_w - 16.0).max(0.0);
            (x, y, w, h)
        }
    }
}

/// 逻辑 x 是否落在左侧文件/Web 预览内容区列内。焦点路由用:点击落在
/// 该列 → 键盘交给 webview;落在别处 → 交回窗口(终端)。
pub fn is_in_preview_column(x: f32, _window_width: f32, state: &ShellState) -> bool {
    if state.left_collapsed {
        return false;
    }
    let left_w = left_zone_width(state);
    match state.left_view {
        LeftView::Web => {
            let start = ICON_RAIL_WIDTH;
            let end = start + left_w;
            x >= start && x < end
        }
        LeftView::Files => {
            let list_w = left_w * state.layout.files_split;
            let start = ICON_RAIL_WIDTH + list_w + DIVIDER_WIDTH;
            let end = ICON_RAIL_WIDTH + left_w;
            x >= start && x < end
        }
    }
}

/// 窗口整体逻辑像素尺寸 → 终端 pane 的可用像素尺寸。终端只在右侧视图是
/// `Agent` 且未收起时可见；否则返回零尺寸(main.rs 的调用方在这种情况下
/// 本就不会真的用这个尺寸去 resize 一个不可见的终端，返回零是安全兜底)。
pub fn terminal_pane_pixel_size(
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32) {
    if state.right_collapsed || state.right_view != RightView::Agent {
        return (0.0, 0.0);
    }
    let right_w = right_zone_width(window_width, state);
    let content_w = right_w * (1.0 - state.layout.agent_split);
    let pane_width = (content_w - CHROME_WIDTH_PX).max(0.0);
    let pane_height =
        (window_height - TOP_BAR_HEIGHT - STATUS_BAR_HEIGHT - CHROME_HEIGHT_PX).max(0.0);
    (pane_width, pane_height)
}
```

- [ ] **Step 2: `Workspace` 字段收尾——删 `old_layout`，`shell_state()` 访问器取代 `layout()`**

删除 `Workspace` 结构体里 Task 1 加的：

```rust
    #[allow(dead_code)]
    old_layout: PanelLayout,
```

两处构造函数里对应删掉 `old_layout: PanelLayout::default(),` 那一行。

把 `impl Workspace` 里的：

```rust
    /// 当前四栏宽度状态(main.rs 拖拽追踪/持久化用;`Copy` 类型直接按值返回)。
    pub fn layout(&self) -> PanelLayout {
        self.layout
    }
```

改为：

```rust
    /// 当前外壳几何状态快照(main.rs 拖拽追踪/离屏几何计算用;`Copy`
    /// 类型直接按值返回)。
    pub fn shell_state(&self) -> ShellState {
        ShellState {
            layout: self.shell_layout,
            left_view: self.left_view,
            left_collapsed: self.left_collapsed,
            right_view: self.right_view,
            right_collapsed: self.right_collapsed,
        }
    }
```

新增只读 getter（紧邻 `dragging_divider` 附近）：

```rust
    pub fn left_view(&self) -> LeftView {
        self.left_view
    }

    pub fn right_view(&self) -> RightView {
        self.right_view
    }

    pub fn maximized(&self) -> Option<MaximizedPane> {
        self.maximized
    }
```

- [ ] **Step 3: `Message`/`update()`——图标选择 + 改 `ColumnDrag`/`ColumnDragEnd`**

在 `Message` 枚举里紧邻 `ColumnDragEnd,` 之后新增：

```rust
    /// 点击左图标栏某图标:已是当前视图则切换收起态,否则切到该视图并展开。
    LeftIconSelect(LeftView),
    /// 同上,右图标栏。
    RightIconSelect(RightView),
```

把 `update()` 里的：

```rust
            Message::ColumnDrag {
                window_width,
                logical_x,
            } => {
                if let Some(divider) = self.dragging {
                    self.layout = apply_column_drag(self.layout, divider, window_width, logical_x);
                }
            }
            Message::ColumnDragEnd => {
                self.dragging = None;
                let layout = self.layout;
                self.handle.spawn(async move {
                    if let Err(e) = layout::save(&layout) {
                        tracing::warn!("四栏布局写盘失败: {e}");
```

改为：

```rust
            Message::ColumnDrag {
                window_width,
                logical_x,
            } => {
                if let Some(divider) = self.dragging {
                    let state = self.shell_state();
                    self.shell_layout = apply_column_drag(state, divider, window_width, logical_x);
                }
            }
            Message::ColumnDragEnd => {
                self.dragging = None;
                let layout = self.shell_layout;
                self.handle.spawn(async move {
                    if let Err(e) = layout::save(&layout) {
                        tracing::warn!("外壳布局写盘失败: {e}");
```

（这段后面还有 `}` 收尾和 `});` 的异步 spawn 结构，原样保留，只改了上面两行涉及 `self.layout`/`layout` 变量名和一处日志文案。）

紧邻新的 `Message::ColumnDragEnd` 分支之后新增：

```rust
            Message::LeftIconSelect(v) => {
                if self.left_view == v {
                    self.left_collapsed = !self.left_collapsed;
                } else {
                    self.left_view = v;
                    self.left_collapsed = false;
                }
            }
            Message::RightIconSelect(v) => {
                if self.right_view == v {
                    self.right_collapsed = !self.right_collapsed;
                } else {
                    self.right_view = v;
                    self.right_collapsed = false;
                }
            }
```

- [ ] **Step 4: 删 `AiView`/`Message::AiViewSwitch`/`ws.ai_view`**

删除 `pub enum AiView { Conversations, Agents }`（及其 `#[default]` 派生，若原定义带 `Default` derive 一并删）。

删除 `Message::AiViewSwitch(AiView),` 变体，以及 `update()` 里对应的 `Message::AiViewSwitch(v) => { self.ai_view = v; }`（若原实现不是这一行字面写法，按实际读到的删除该分支）。

删除 `Workspace` 结构体的 `ai_view: AiView,` 字段，以及两处构造函数里的 `ai_view: AiView::default(),`（或等价初始化行）。

- [ ] **Step 5: `preview.rs`——删 `TabKind::Review`/`open_review`/`review_active`**

把 `crates/dozer-app/src/preview.rs` 里的：

```rust
pub enum TabKind {
    File(PathBuf),
    Web {
        url: String,
    },
    /// 验收 tab（P1f）:不产 webview,内容由 iced 直绘。
    Acceptance,
    /// 会话审阅 tab（P1i）:不产 webview,内容由 iced 直绘。
    Review,
}
```

改为：

```rust
pub enum TabKind {
    File(PathBuf),
    Web {
        url: String,
    },
    /// 验收 tab（P1f）:不产 webview,内容由 iced 直绘。
    Acceptance,
}
```

删除整个：

```rust
    /// 打开会话审阅 tab:已存在则激活复用（全局至多一个）。
    // 过渡期:T3 ConversationOpen 接线前无调用方（终端审阅按钮已移除）。
    #[allow(dead_code)]
    pub fn open_review(&mut self) -> usize {
        if let Some((idx, tab)) = self
            .tabs
            .iter()
            .enumerate()
            .find(|(_, t)| t.kind == TabKind::Review)
        {
            let id = tab.id;
            self.active = idx;
            return id;
        }
        self.push_tab(TabKind::Review, "审阅".to_string())
    }

    pub fn review_active(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|t| t.kind == TabKind::Review)
    }
```

把 `active_webview_id` 里的：

```rust
            TabKind::Acceptance | TabKind::Review => None,
```

改为：

```rust
            TabKind::Acceptance => None,
```

同理，`preview.rs` 里另一处 `TabKind::Acceptance | TabKind::Review => return None,`（约第 248 行一带，从属于计算标签宽度或类似逻辑的 match）改为 `TabKind::Acceptance => return None,`。

**删除 `preview.rs` 测试里对 `open_review`/`review_active`/`TabKind::Review` 的引用**——运行 `cargo build -p dozer-app --tests 2>&1 | grep "preview.rs"` 找出所有报错行号，逐一删掉那几个测试函数（不是注释掉，是整个删除——它们测的行为已经不存在）。

- [ ] **Step 6: `workspace.rs`——`ConversationOpen` 去掉 `open_review()` 调用**

把 `Message::ConversationOpen(path) => { ... }` 处理块末尾的：

```rust
                self.preview.open_review();
```

整行删除。上面几行 `self.review = Some(ReviewView { ... });` 原样保留——这才是驱动"对话审阅"内容的真正状态，`open_review()` 调用只是旧模型里"顺便在共享 tab 条里也弹一个 tab"的动作，新模型下"对话审阅"是独立面板，不需要它。

- [ ] **Step 7: `project_pane`/`preview_pane`/`terminal_pane` 改成接收 `width: f32` 参数**

把 `fn project_pane(ws: &Workspace) -> Element<...>` 签名改成：

```rust
fn project_pane(ws: &Workspace, width: f32) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
```

函数体内唯一涉及宽度的一行：

```rust
    container(column![body, project_status_bar(ws)])
        .width(Length::Fixed(ws.layout.project_col_width))
        .height(Length::Fill)
        .into()
```

改为：

```rust
    container(column![body, project_status_bar(ws)])
        .width(Length::Fixed(width))
        .height(Length::Fill)
        .into()
```

同理，`fn preview_pane(ws: &Workspace) -> Element<...>` 签名加 `width: f32` 参数，函数体内：

```rust
    let preview_portion = (ws.layout.preview_ratio * 10_000.0).round() as u16;
    container(content.padding(8))
        .width(Length::FillPortion(preview_portion))
        .height(Length::Fill)
```

改为：

```rust
    container(content.padding(8))
        .width(Length::Fixed(width))
        .height(Length::Fill)
```

（不再需要 `preview_portion` 这个中间变量——新模型下宽度是调用方算好直接传入的固定值，不是"跟兄弟栏共享 Fill 比例"，删掉这行。）

同理，`fn terminal_pane(ws: &Workspace) -> Element<...>` 加 `width: f32` 参数，函数体内：

```rust
    let terminal_portion = ((1.0 - ws.layout.preview_ratio) * 10_000.0).round() as u16;
    container(column![body, terminal_status_bar(ws)])
        .width(Length::FillPortion(terminal_portion))
        .height(Length::Fill)
        .into()
```

改为：

```rust
    container(column![body, terminal_status_bar(ws)])
        .width(Length::Fixed(width))
        .height(Length::Fill)
        .into()
```

- [ ] **Step 8: 拆 `ai_pane` 成 `agent_list_pane`/`conversation_list_pane`**

把现有 `fn ai_pane(ws: &Workspace) -> Element<...> { ... }` 整个函数删除，替换成两个新函数。

`conversation_list_pane` 取自原 `ai_pane` 里"顶部标题行 + `AiView::Conversations` 那个 match 分支的完整内容"（对话计数、空态提示、按 `is_current_conversation`/`mtime` 排序、每条对话的卡片渲染），去掉原来的 `mk_pill`/两个 pill 按钮那一段（图标栏已经替代了这个切换 UI），外层容器宽度参数化：

```rust
fn conversation_list_pane(ws: &Workspace, width: f32) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut content = column![
        row![
            text("对话").size(14).color(theme::CREAM),
            text(
                ws.project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未打开项目".into())
            )
            .size(12)
            .color(theme::DIM),
        ]
        .spacing(8)
    ]
    .spacing(8);

    let opens = ws.open_transcript_paths();
    let active_n = ws
        .conversations
        .iter()
        .filter(|c| conversation::is_current_conversation(&c.path, &opens))
        .count();
    content = content.push(
        row![
            text("对话").size(11).color(theme::DIM),
            text(format!("{} 条 · {} 活跃", ws.conversations.len(), active_n))
                .size(11)
                .color(theme::DIM),
        ]
        .spacing(6),
    );
    if ws.conversations.is_empty() {
        content = content.push(text("暂无对话记录").size(13).color(theme::DIM));
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mut ordered: Vec<&ConversationMeta> = ws.conversations.iter().collect();
    ordered.sort_by_key(|c| conversation::is_current_conversation(&c.path, &opens) as u8);
    ordered.reverse();
    for c in ordered {
        let current = conversation::is_current_conversation(&c.path, &opens);
        let sub = if current {
            format!(
                "● 当前 · {}",
                conversation_sub(&c.agent, c.modified_ms, c.size_bytes, now_ms)
            )
        } else {
            conversation_sub(&c.agent, c.modified_ms, c.size_bytes, now_ms)
        };
        let sub_color = if current { theme::GREEN } else { theme::DIM };
        let card = button(
            column![
                text(c.title.clone()).size(13).color(theme::CREAM),
                text(sub).size(10).color(sub_color),
            ]
            .spacing(4),
        )
        .on_press(Message::ConversationOpen(c.path.clone()))
        .width(Length::Fill)
        .padding(10)
        .style(move |_t, _s| button::Style {
            background: Some(theme::CARD.into()),
            text_color: theme::CREAM,
            border: Border {
                color: if current { theme::GOLD } else { theme::BORDER },
                width: 1.0,
                radius: 8.0.into(),
            },
            ..button::Style::default()
        });
        content = content.push(card);
    }

    container(content.padding(12))
        .width(Length::Fixed(width))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            ..container::Style::default()
        })
        .into()
}
```

`agent_list_pane` 取自原 `AiView::Agents` 分支（目前只有一句占位文案），同样标题行+参数化宽度：

```rust
fn agent_list_pane(ws: &Workspace, width: f32) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let content = column![
        row![
            text("Agent").size(14).color(theme::CREAM),
            text(
                ws.project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未打开项目".into())
            )
            .size(12)
            .color(theme::DIM),
        ]
        .spacing(8),
        text("Agents（后续）").size(13).color(theme::DIM),
    ]
    .spacing(8);

    container(content.padding(12))
        .width(Length::Fixed(width))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            ..container::Style::default()
        })
        .into()
}
```

- [ ] **Step 9: 新增 `review_content_pane`**

新增函数，直接从 `ws.review` 取数据渲染，不经过 `ws.preview` 的 tab 系统。复用已有的 `review_content(content, ws)` 辅助函数（它接收一个 `column!` 累加器并往里 push 审阅内容，原来是 `preview_pane` 在用；签名不用改，直接复用）：

```rust
fn review_content_pane(ws: &Workspace, width: f32) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let header = row![text("会话审阅").size(13).color(theme::CREAM)].spacing(4);
    let mut content = column![header].spacing(4);

    if ws.review.is_some() {
        content = review_content(content, ws);
    } else {
        content = content.push(
            container(
                text("暂无审阅内容——点击左侧对话列表中的对话开始审阅")
                    .size(14)
                    .color(theme::DIM),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        );
    }

    container(content.padding(8))
        .width(Length::Fixed(width))
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            ..container::Style::default()
        })
        .into()
}
```

（`review_content` 函数本身此前是 `preview_pane` 专用的私有辅助函数，如果它的可见性/位置导致这里调不到，把它的定义原样保留在原位置，本步骤只是新增一个新的调用方，不改 `review_content` 自己的签名。）

- [ ] **Step 10: 新增图标栏渲染函数**

```rust
/// 单个图标栏按钮:激活态金色描边+底色，未激活态纯图标。
fn rail_icon_button<'a>(
    icon: icons::IconKind,
    active: bool,
    msg: Message,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let color = if active { theme::GOLD } else { theme::DIM };
    let inner = container(icons::view(icon, 18.0, color))
        .width(Length::Fixed(36.0))
        .height(Length::Fixed(36.0))
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: if active {
                Some(theme::CARD.into())
            } else {
                None
            },
            border: if active {
                Border {
                    color: theme::GOLD,
                    width: 1.0,
                    radius: 8.0.into(),
                }
            } else {
                Border::default()
            },
            ..container::Style::default()
        });
    button(inner)
        .on_press(msg)
        .style(|_t, _s| button::Style {
            background: None,
            ..button::Style::default()
        })
        .into()
}

fn left_icon_rail(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let content = column![
        rail_icon_button(
            icons::IconKind::Folder,
            ws.left_view == LeftView::Files,
            Message::LeftIconSelect(LeftView::Files),
        ),
        rail_icon_button(
            icons::IconKind::Globe,
            ws.left_view == LeftView::Web,
            Message::LeftIconSelect(LeftView::Web),
        ),
    ]
    .spacing(12)
    .padding([16, 0, 0, 0])
    .align_x(iced_widget::core::Alignment::Center);

    container(content)
        .width(Length::Fixed(ICON_RAIL_WIDTH))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            ..container::Style::default()
        })
        .into()
}

fn right_icon_rail(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let content = column![
        rail_icon_button(
            icons::IconKind::Bot,
            ws.right_view == RightView::Agent,
            Message::RightIconSelect(RightView::Agent),
        ),
        rail_icon_button(
            icons::IconKind::MessageSquare,
            ws.right_view == RightView::Conversations,
            Message::RightIconSelect(RightView::Conversations),
        ),
    ]
    .spacing(12)
    .padding([16, 0, 0, 0])
    .align_x(iced_widget::core::Alignment::Center);

    container(content)
        .width(Length::Fixed(ICON_RAIL_WIDTH))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            ..container::Style::default()
        })
        .into()
}
```

- [ ] **Step 11: 新增左右面板区组合函数**

```rust
fn left_panel_area(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    if ws.left_collapsed {
        return column![].into();
    }
    let total = ws.shell_layout.left_width;
    match ws.left_view {
        LeftView::Files => {
            let list_w = total * ws.shell_layout.files_split;
            let content_w = total - list_w - DIVIDER_WIDTH;
            row![
                project_pane(ws, list_w),
                divider_bar(Divider::LeftPairSplit),
                preview_pane(ws, content_w),
            ]
            .into()
        }
        LeftView::Web => preview_pane(ws, total),
    }
}

fn right_panel_area(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    if ws.right_collapsed {
        return column![].into();
    }
    match ws.right_view {
        RightView::Agent => {
            // 右面板区总宽是剩余空间(Fill)，这里没有一个现成的 f32 总宽——
            // 用 Length::FillPortion 让 list/content 两块按 agent_split 分配
            // 剩余空间，不需要预先知道总像素数(与左侧不同，左侧总宽是持久化
            // 的固定值，右侧从来都是"拿剩下的")。
            let list_portion = (ws.shell_layout.agent_split * 10_000.0).round() as u16;
            let content_portion = ((1.0 - ws.shell_layout.agent_split) * 10_000.0).round() as u16;
            row![
                container(agent_list_pane(ws, 0.0))
                    .width(Length::FillPortion(list_portion)),
                divider_bar(Divider::RightPairSplit),
                container(terminal_pane(ws, 0.0))
                    .width(Length::FillPortion(content_portion)),
            ]
            .into()
        }
        RightView::Conversations => {
            let list_portion = (ws.shell_layout.conversations_split * 10_000.0).round() as u16;
            let content_portion =
                ((1.0 - ws.shell_layout.conversations_split) * 10_000.0).round() as u16;
            row![
                container(conversation_list_pane(ws, 0.0))
                    .width(Length::FillPortion(list_portion)),
                divider_bar(Divider::RightPairSplit),
                container(review_content_pane(ws, 0.0))
                    .width(Length::FillPortion(content_portion)),
            ]
            .into()
        }
    }
}
```

**已知的一处不一致，故意留着**：右面板区两个配对（Agent/对话）用 `Length::FillPortion` 包一层 `container` 再套外层宽度为 `0.0` 的 pane 函数（`agent_list_pane(ws, 0.0)` 的 `0.0` 参数其实被内层 `Length::Fixed(0.0)` 设置了又被外层 `container.width(FillPortion(..))` 覆盖——`Fixed` 宽度设在子元素上会被父级 `FillPortion` 覆盖，这是 iced 的正常行为，子元素自己声明的固定宽度对 `FillPortion` 父级不生效）。这与左侧"先算好精确像素宽度再传 `Length::Fixed`"的做法不统一，是本任务为了避免"右面板区总宽度"这个当前完全没有的量而采用的折中——**右面板区从来没有一个持久化的总像素宽度**（它恒为 Fill），要严格算出"剩余空间的具体像素数"需要知道窗口宽度，而这两个函数目前拿不到窗口宽度参数。如果 Step 6 之后的真机验收发现 `agent_list_pane`/`conversation_list_pane`/`terminal_pane`/`review_content_pane` 内部有依赖精确 `width: f32` 数值做像素级计算的地方（目前读到的代码里，这四个函数拿到的 `width` 只用来设外层容器宽度，不做其他计算，所以传 `0.0` 是安全的），需要改造成传窗口宽度算出的精确值——先记录这个已知簡化，不在本任务展开修。

- [ ] **Step 12: 重写 `view()`**

把 `pub fn view(&self) -> Element<...> { ... }` 里的：

```rust
        let top = top_bar(self);
        let col1 = project_pane(self);
        let col2 = preview_pane(self);
        let col3 = terminal_pane(self);
        let col4 = ai_pane(self);
        let base = column![
            top,
            row![
                col1,
                divider_bar(Divider::ProjectPreview),
                col2,
                divider_bar(Divider::PreviewTerminal),
                col3,
                divider_bar(Divider::TerminalAi),
                col4
            ]
        ];
```

改为：

```rust
        let top = top_bar(self);
        let body = row![
            left_icon_rail(self),
            left_panel_area(self),
            divider_bar(Divider::LeftRight),
            right_panel_area(self),
            right_icon_rail(self),
        ];
        let base = column![top, body];
```

`view()` 剩下的 `stack!` 逻辑（`tree_delete_confirm`/`context_menu` 两层判断）原样保留，不用改——它们包的是整个 `base`，跟内部是四栏还是新外壳无关。

- [ ] **Step 13: `ime_cursor_area` 适配新几何**

把 `pub fn ime_cursor_area(&self, window_w: f32, window_h: f32) -> (f32, f32, f32) { ... }` 里的：

```rust
        if self.preview.addr_editing() || self.acceptance_comment_editing() {
            return (
                self.layout.project_col_width + 12.0,
                TOP_BAR_HEIGHT + PREVIEW_CHROME_TOP_PX,
                20.0,
            );
        }
        let (pane_w, pane_h) = terminal_pane_pixel_size(window_w, window_h, &self.layout);
        let cell_w = pane_w / self.cols.max(1) as f32;
        let line_h = pane_h / self.rows.max(1) as f32;
        let fill_width = (window_w
            - self.layout.project_col_width
            - self.layout.ai_col_width
            - 3.0 * DIVIDER_WIDTH)
            .max(0.0);
        let x0 = self.layout.project_col_width
            + 2.0 * DIVIDER_WIDTH
            + fill_width * self.layout.preview_ratio
            + 8.0;
```

改为：

```rust
        let state = self.shell_state();
        if self.preview.addr_editing() || self.acceptance_comment_editing() {
            let (bx, by, _bw, _bh) = preview_content_bounds(window_w, window_h, &state);
            return (bx + 4.0, by, 20.0);
        }
        let (pane_w, pane_h) = terminal_pane_pixel_size(window_w, window_h, &state);
        let cell_w = pane_w / self.cols.max(1) as f32;
        let line_h = pane_h / self.rows.max(1) as f32;
        let right_w = right_zone_width(window_w, &state);
        let list_w = right_w * state.layout.agent_split;
        let x0 = window_w - ICON_RAIL_WIDTH - right_w + list_w + DIVIDER_WIDTH + 8.0;
```

（原来的 `x0` 公式是"从项目栏右边算到预览:终端分界"，本质是"终端 pane 左边缘在哪"；新公式是"从窗口右边算到右面板区左边缘，再加 Agent 配对里列表部分的宽度、再加一条分隔线"——两者都是"终端内容区左边缘的 x 坐标"，只是新模型下终端锚定在右面板区而不是紧跟项目栏。函数剩余部分（`y0`/`col`/`row` 计算）不涉及 `self.layout` 字段，原样保留不用改。）

- [ ] **Step 14: main.rs 适配**

把 `crates/dozer-app/src/main.rs` 里所有 `&workspace.layout()` 的调用点改成 `&workspace.shell_state()`（`workspace.layout()` 这个方法已经在 Task 3 Step 2 被 `shell_state()` 取代，若还留有旧调用点编译会直接报错，按报错逐一改）。具体两处已知调用点：

第 297 行一带（`is_in_preview_column` 调用）：

```rust
                        if workspace::is_in_preview_column(
                            logical_x,
                            logical_w,
                            &workspace.layout(),
                        ) {
```

改为：

```rust
                        if workspace::is_in_preview_column(
                            logical_x,
                            logical_w,
                            &workspace.shell_state(),
                        ) {
```

第 464 行一带（`sync_previews` 里的 `preview_content_bounds` 调用）：

```rust
            let (x, y, w, h) =
                workspace::preview_content_bounds(logical_w, logical_h, &workspace.layout());
```

改为：

```rust
            let (x, y, w, h) =
                workspace::preview_content_bounds(logical_w, logical_h, &workspace.shell_state());
```

第 723 行、第 963 行一带两处 `terminal_pane_pixel_size` 调用同理，`&workspace.layout()` 全部换成 `&workspace.shell_state()`。

- [ ] **Step 15: 编译 + 测试 + clippy + fmt**

Run: `cargo build -p dozer-app 2>&1 | tail -80`
Expected: 编译通过。这一步极可能第一次跑不过——按报错逐条修，常见的几类：
  - 某处仍写着旧字段名 `self.layout`（现在叫 `self.shell_layout`）或旧类型 `PanelLayout`。
  - 某个 pane 函数调用点漏了新加的 `width` 参数。
  - `ai_pane`/`AiView` 还有遗留引用（比如某处 UI 代码里还在 `match ws.ai_view`）。
  - `preview.rs`/`workspace.rs` 的测试模块里有对 `TabKind::Review`/`open_review`/`AiView`/`Divider::ProjectPreview` 等已删符号的引用。

Run: `cargo test -p dozer-app 2>&1 | tail -60`
Expected: 通过（测试数量会比之前少——删掉了 `preview.rs` 里 Review 相关的测试、旧 `apply_column_drag`/`preview_content_bounds`/`is_in_preview_column`/`terminal_pane_pixel_size` 相关的旧几何测试。若有旧几何测试引用了已删除的 `MIN_PROJECT_COL_WIDTH` 等常量或旧函数签名，同样需要删除或改写成新签名——改写优先于删除：如果一条旧测试的意图（比如"拖到下限会被 clamp 住"）在新几何模型下仍然成立，改写成调用新函数、断言新字段，不要图省事直接删掉验证意图）。

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean`
Run: `cargo fmt -p dozer-app -- --check`
Expected: clean / 无输出。

- [ ] **Step 16: 真机目测（留用户）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：窗口呈现"左图标栏(文件/Web 两个图标) + 左面板区 + 右面板区 + 右图标栏(Agent/对话两个图标)"，无常驻中间区；默认左=文件列表(项目树+文件预览并排)、右=Agent(占位列表+终端并排)；点左图标栏"Web"→左侧变成单个预览面板占满左面板区宽度(项目树消失)；再点一次"文件列表"→项目树回来；点右图标栏"对话"→右侧变成"对话列表+对话审阅"并排，点对话列表里的一条对话，审阅内容立刻在紧邻的右侧出现(不再"隔一条街")；再点一次当前已激活的图标(比如已经是"文件列表"时再点"文件列表")→左侧整个收起，右侧占满剩余宽度；左右面板区之间、以及每侧配对视图内部的列表:内容分隔线均可拖拽且宽度会记住。

- [ ] **Step 17: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/preview.rs crates/dozer-app/src/main.rs
git commit -m "feat(shell): 图标栏驱动的左右面板区取代四栏固定布局(核心切换)"
```

---

### Task 4: 视图选择 + 收起态持久化

**Files:**
- Modify: `crates/dozer-app/src/layout.rs`（持久化结构体新增视图选择/收起态字段）
- Modify: `crates/dozer-app/src/workspace.rs`（启动读取、`ColumnDragEnd`/图标切换时写盘）

**Interfaces:**
- Consumes: Task 3 的 `ShellState`/`LeftView`/`RightView`。
- Produces: `ShellLayout` 扩展字段 `left_view`/`right_view`/`left_collapsed`/`right_collapsed`（复用同一个持久化文件，不新开一份）。

设计文档 §2.2 要求"每侧当前激活的图标持久化"——本任务把这四个状态并进已有的 `ShellLayout`/`layout.json`，不单独起一套持久化机制。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/layout.rs` 测试模块里追加：

```rust
    #[test]
    fn shell_layout_persists_view_selection_and_collapse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shell_layout.json");
        let layout = ShellLayout {
            left_width: 500.0,
            files_split: 0.4,
            agent_split: 0.35,
            conversations_split: 0.45,
            left_view: LeftView::Web,
            right_view: RightView::Conversations,
            left_collapsed: true,
            right_collapsed: false,
        };
        save_to(&path, &layout).unwrap();
        assert_eq!(load_from(&path), layout);
    }
```

在文件顶部加 `use crate::workspace::{LeftView, RightView, ShellLayout};`（若已有 `use crate::workspace::ShellLayout;` 则改成这一行合并导入）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer layout::tests::shell_layout_persists`
Expected: FAIL（`ShellLayout` 还没有 `left_view` 等字段）。

- [ ] **Step 3: `ShellLayout` 新增字段 + `Default`**

把 `crates/dozer-app/src/workspace.rs` 里的：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShellLayout {
    pub left_width: f32,
    pub files_split: f32,
    pub agent_split: f32,
    pub conversations_split: f32,
}

impl Default for ShellLayout {
    fn default() -> Self {
        Self {
            left_width: 640.0,
            files_split: 0.35,
            agent_split: 0.4,
            conversations_split: 0.4,
        }
    }
}
```

改为：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShellLayout {
    pub left_width: f32,
    pub files_split: f32,
    pub agent_split: f32,
    pub conversations_split: f32,
    pub left_view: LeftView,
    pub right_view: RightView,
    pub left_collapsed: bool,
    pub right_collapsed: bool,
}

impl Default for ShellLayout {
    fn default() -> Self {
        Self {
            left_width: 640.0,
            files_split: 0.35,
            agent_split: 0.4,
            conversations_split: 0.4,
            left_view: LeftView::Files,
            right_view: RightView::Agent,
            left_collapsed: false,
            right_collapsed: false,
        }
    }
}
```

给 `LeftView`/`RightView` 的 `#[derive(...)]` 加上 `Serialize, Deserialize`（它们现在要跟着 `ShellLayout` 一起走 serde）：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LeftView {
    Files,
    Web,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RightView {
    Agent,
    Conversations,
}
```

- [ ] **Step 4: 启动读取 + 图标切换/收起时写盘**

`Workspace` 两处构造函数（`bootstrap`/`with_daemon_error`）里，把：

```rust
            shell_layout: layout::load(),
            left_view: LeftView::Files,
            right_view: RightView::Agent,
            left_collapsed: false,
            right_collapsed: false,
```

改为直接从读到的 `shell_layout` 取这四个字段，避免"读盘值"和"字段初始值"两处独立写、容易漂移：

```rust
            left_view: shell_layout.left_view,
            right_view: shell_layout.right_view,
            left_collapsed: shell_layout.left_collapsed,
            right_collapsed: shell_layout.right_collapsed,
            shell_layout,
```

（这要求在这段字面量构造之前先 `let shell_layout = layout::load();`，把原来内联的 `layout::load()` 调用提出来存成局部变量，两处构造函数都要这样改。注意 Rust 结构体字面量里字段声明顺序不影响语义，但 `shell_layout` 这一行要放在用到它的字段之后才符合"先声明局部变量再用"的直觉可读性——实际写的时候 `let shell_layout = layout::load();` 是一条独立语句，放在整个 `Self { ... }` 字面量之前即可，字面量内部字段顺序随意。）

在 `update()` 里，`Message::ColumnDragEnd` 已经会异步写盘 `self.shell_layout`——这已经覆盖了宽度/分割比例的持久化。新增：`Message::LeftIconSelect`/`RightIconSelect` 分支末尾也要触发写盘（图标切换应该立即持久化，不必等用户去拖一次分隔线才顺带存上）：

```rust
            Message::LeftIconSelect(v) => {
                if self.left_view == v {
                    self.left_collapsed = !self.left_collapsed;
                } else {
                    self.left_view = v;
                    self.left_collapsed = false;
                }
                self.spawn_shell_layout_save();
            }
            Message::RightIconSelect(v) => {
                if self.right_view == v {
                    self.right_collapsed = !self.right_collapsed;
                } else {
                    self.right_view = v;
                    self.right_collapsed = false;
                }
                self.spawn_shell_layout_save();
            }
```

新增私有方法（放在 `spawn_new_tab` 附近即可）：

```rust
    /// 把 `left_view`/`right_view`/`left_collapsed`/`right_collapsed` 同步进
    /// `shell_layout` 再异步写盘。图标切换/收起要立即持久化，不能只靠
    /// `ColumnDragEnd` 顺带存(用户可能从没拖过分隔线)。
    fn spawn_shell_layout_save(&mut self) {
        self.shell_layout.left_view = self.left_view;
        self.shell_layout.right_view = self.right_view;
        self.shell_layout.left_collapsed = self.left_collapsed;
        self.shell_layout.right_collapsed = self.right_collapsed;
        let layout = self.shell_layout;
        self.handle.spawn(async move {
            if let Err(e) = layout::save(&layout) {
                tracing::warn!("外壳布局写盘失败: {e}");
            }
        });
    }
```

`Message::ColumnDragEnd` 现有的写盘逻辑保持不变即可（它写的是 `self.shell_layout`，此时 `left_view` 等字段已经通过 `spawn_shell_layout_save` 同步过，不会写出陈旧值——但为防止"用户先拖分隔线、还没切过图标"时 `shell_layout.left_view` 字段是初始默认值而不是当前 `self.left_view` 的边界情况，`ColumnDragEnd` 的写盘前也加一行同步）：

```rust
            Message::ColumnDragEnd => {
                self.dragging = None;
                self.shell_layout.left_view = self.left_view;
                self.shell_layout.right_view = self.right_view;
                self.shell_layout.left_collapsed = self.left_collapsed;
                self.shell_layout.right_collapsed = self.right_collapsed;
                let layout = self.shell_layout;
                self.handle.spawn(async move {
                    if let Err(e) = layout::save(&layout) {
                        tracing::warn!("外壳布局写盘失败: {e}");
                    }
                });
            }
```

- [ ] **Step 5: 编译 + 测试 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。

- [ ] **Step 6: 真机目测（留用户）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：切到"Web"图标、收起左侧、退出 app、重新 `cargo run` → 重开后左图标栏仍是"Web"选中、仍是收起状态，不是回到默认"文件列表+展开"。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/layout.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(shell): 图标选择/收起态持久化(并入现有 ShellLayout 磁盘文件)"
```

---

### Task 5: Pane 放大

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`Message` 新增放大相关变体；`update()` 新增分支；`preview_pane`/`terminal_pane`/`review_content_pane` 顶部加放大按钮；新增 `maximize_overlay`；`view()` 接入放大层）

**Interfaces:**
- Consumes: Task 1 的 `MaximizedPane`；Task 2 的 `IconKind::Maximize`；Task 3 的 `ICON_RAIL_WIDTH`/`left_panel_area`/`right_panel_area`。
- Produces: `Message::MaximizeToggle(MaximizedPane)`、`Message::MaximizeClose`。

- [ ] **Step 1: `Message`/`update()` 新增**

在 `Message` 枚举里紧邻 `RightIconSelect(RightView),` 之后新增：

```rust
    /// 点击某内容 pane 的放大按钮:已放大同一侧则还原,否则放大该侧。
    MaximizeToggle(MaximizedPane),
    /// 点击放大态背后的变暗遮罩:退出放大。
    MaximizeClose,
```

在 `update()` 里紧邻 `Message::RightIconSelect` 分支之后新增：

```rust
            Message::MaximizeToggle(which) => {
                self.maximized = if self.maximized == Some(which) {
                    None
                } else {
                    Some(which)
                };
            }
            Message::MaximizeClose => {
                self.maximized = None;
            }
```

- [ ] **Step 2: 给内容类 pane 的 tab 栏/头部加放大按钮**

`preview_pane`（文件/Web 预览内容，位于左面板区）：把 `tab_bar` 那一行（`let tab_bar = row![left_arrow, clipped, right_arrow, open_btn] ...`）改成末尾追加放大按钮：

```rust
    let maximize_btn = button(icons::view(icons::IconKind::Maximize, 14.0, theme::DIM))
        .on_press(Message::MaximizeToggle(MaximizedPane::Left))
        .style(|_t, _s| button::Style {
            background: None,
            ..button::Style::default()
        });
    let tab_bar = row![left_arrow, clipped, right_arrow, open_btn, maximize_btn]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
```

`terminal_pane`（终端内容，位于右面板区 Agent 配对）：把 `let mut content = column![tab_bar(ws)].spacing(4);` 那一行的 `tab_bar(ws)` 换成一个带放大按钮的版本——找到 `tab_bar` 函数定义（终端 tab 条，与 `preview_pane` 的 tab 条是两个不同函数，不要改混），在它构造的最终 `row!`/返回值末尾同样追加一个 `maximize_btn`（`Message::MaximizeToggle(MaximizedPane::Right)`），做法与上面 `preview_pane` 一致。

`review_content_pane`（Task 3 新增）：把 Step 9 写的：

```rust
    let header = row![text("会话审阅").size(13).color(theme::CREAM)].spacing(4);
```

改为：

```rust
    let maximize_btn = button(icons::view(icons::IconKind::Maximize, 14.0, theme::DIM))
        .on_press(Message::MaximizeToggle(MaximizedPane::Right))
        .style(|_t, _s| button::Style {
            background: None,
            ..button::Style::default()
        });
    let header = row![
        text("会话审阅").size(13).color(theme::CREAM),
        iced_widget::space::horizontal(),
        maximize_btn,
    ]
    .spacing(4);
```

- [ ] **Step 3: `maximize_overlay` 渲染**

```rust
/// 放大态浮层:两条图标栏之间的整个内容区变暗+背景虚化，放大的那一侧
/// 内容(左/右面板区，含其内部列表:内容子分隔线，原样渲染，只是占满整个
/// 中间区域)金色描边突出。点变暗区域(放大内容之外的部分)退出放大。
fn maximize_overlay(
    ws: &Workspace,
    which: MaximizedPane,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let inner = match which {
        MaximizedPane::Left => left_panel_area(ws),
        MaximizedPane::Right => right_panel_area(ws),
    };
    let bordered = container(inner).style(move |_t: &iced_widget::Theme| container::Style {
        border: Border {
            color: theme::GOLD,
            width: 1.5,
            radius: 10.0.into(),
        },
        ..container::Style::default()
    });
    let dim_bg = MouseArea::new(
        container(bordered)
            .padding(40)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(Color { r: 0.0, g: 0.0, b: 0.0, a: 0.55 }.into()),
                ..container::Style::default()
            }),
    )
    .on_press(Message::MaximizeClose);

    row![
        iced_widget::space::Space::new().width(Length::Fixed(ICON_RAIL_WIDTH)),
        dim_bg,
        iced_widget::space::Space::new().width(Length::Fixed(ICON_RAIL_WIDTH)),
    ]
    .into()
}
```

（`iced_widget::core::Color` 需要在文件顶部已有 `use` 覆盖——本文件已大量使用 `Color`，若尚未导入按编译器提示补 `use iced_widget::core::Color;`。放大态没有做设计文档提到的"BACKGROUND_BLUR 背景虚化"——iced 0.14 没有现成的背景虚化效果 API，这里只做了半透明黑遮罩，没有虚化。这是从 Figma 视觉稿到真实 iced 渲染管线的一处已知落差，不在本任务范围内解决，若需要虚化效果需要评估 iced 的 shader/自定义 renderer 能力，是比这个任务大得多的另一件事。）

- [ ] **Step 4: `view()` 接入放大层**

把 Task 3 Step 12 写的 `view()` 里判断 `tree_delete_confirm`/`context_menu` 的 `stack!` 链，最外层再包一层放大态判断（放大态优先级最高——如果用户在放大态下右键，那一刻的行为本任务不特别设计，`context_menu`/`tree_delete_confirm` 的判断逻辑保持原有优先级不变，只是最终结果外面再套一层放大判断）：

```rust
        let popped = if self.tree_delete_confirm.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectTreeDeleteCancel);
            stack![base, dismiss, delete_confirm_popup(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.context_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectTreeContextMenuClose);
            stack![base, dismiss, context_menu_popup(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            base.into()
        };

        if let Some(which) = self.maximized {
            stack![popped, maximize_overlay(self, which)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            popped
        }
```

（这段替换掉 Task 3 Step 12 里 `view()` 函数体末尾 `if self.tree_delete_confirm.is_some() { ... } else if ... else { base.into() }` 那一整块——把它的返回值先存进 `popped` 局部变量，而不是直接 `return`/表达式收尾，再在外面叠一层放大判断。）

- [ ] **Step 5: 编译 + 测试 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。

- [ ] **Step 6: 真机目测（留用户）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：点文件预览/终端/对话审阅 tab 栏上的放大图标 → 该内容放大到两条图标栏之间的整个区域，金色描边，其他内容(图标栏、另一侧面板区)仍可见但变暗(没有虚化，只有变暗，见上面 Step 3 的已知落差说明)；点变暗区域(放大内容外面)→ 还原；再点一次放大图标(此时已放大)→ 同样还原。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(shell): pane 放大(层叠浮层,背景变暗保留渲染,金色描边)"
```

---

### Task 6: 清理与整体回归

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（摘除本计划全程新增条目上不再需要的 `#[allow(dead_code)]`）

- [ ] **Step 1: 摘除已消费条目的 `dead_code` allow**

Task 1 给 `LeftView`/`RightView`/`MaximizedPane`/`ShellLayout`/`ICON_RAIL_WIDTH`（原 `ShellDivider`，已在 Task 3 改名为 `Divider` 且去掉了 allow）标了 `#[allow(dead_code)]`——Task 3-5 结束后这些全部有真实调用方，逐个摘除对应的 `#[allow(dead_code)]` 属性（`icons.rs` 模块级的 `#![allow(dead_code)]` 不用动，那是仿 `theme.rs` 的永久性豁免，不是本计划引入的）。

- [ ] **Step 2: 全量回归**

Run: `cargo build --all-targets && cargo test --workspace && cargo clippy --all-targets && cargo fmt --all -- --check`
Expected: 整个 workspace（不只是 `dozer-app`）编译/测试/lint 全绿——`preview_content_bounds`/`is_in_preview_column`/`terminal_pane_pixel_size`/`ShellState` 都是 `pub`，理论上没有其他 crate 引用它们（Task 3 之前的 Explore 已确认"blast radius 只在这 3 个文件"），这一步是最终确认，不是走过场。

- [ ] **Step 3: 真机验收清单（留用户，逐条过一遍 Task 3-5 的目测点）**

- [ ] 默认启动:左=文件列表(展开)、右=Agent(展开)，无常驻中间区。
- [ ] 左图标栏"文件列表"↔"Web"切换正确；右图标栏"Agent"↔"对话"切换正确。
- [ ] 点已激活图标 = 收起该侧；再点 = 展开，且展开后仍是收起前的视图。
- [ ] "对话"图标下，点对话列表任意一条，审阅内容立刻出现在紧邻的右侧(不隔终端)。
- [ ] 左右面板区分隔线、每侧配对视图内部分隔线，均可拖拽，松开后刷新 app 仍保留宽度/比例。
- [ ] 图标选择/收起态，刷新 app 后仍保留。
- [ ] 文件预览/终端/对话审阅各自的放大按钮都能放大/还原；放大态下点变暗区域退出。
- [ ] 验收(Acceptance)流程未受影响——从终端交付横幅"进入验收"仍能正常进入，行为与本计划实施前一致。
- [ ] 项目树右键菜单、拖拽分隔线光标、⌘K 一类既有交互未受影响(抽样检查，不用逐项过之前所有计划的验收单)。

- [ ] **Step 4: Commit**（若 Step 1 有实际改动）

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "chore(shell): 摘除已消费条目的 dead_code allow(整体回归收尾)"
```

---

## 自检记录（写计划时）

- **Spec 覆盖**：设计文档 §2.1(图标集/验收不进图标栏)→ Task 3 Step 10-11(图标栏只有 4 个图标，无验收)；§2.2(状态与交互:持久化/再点收起/可拖拽/独立分割比例)→ Task 3(收起/拖拽)+Task 4(持久化)；§4(pane 放大/背后内容变暗保留)→ Task 5；§4.1(默认"所有内容类 pane 都有放大按钮")→ Task 5 Step 2 覆盖了文件预览/终端/对话审阅三个，Agent 列表/项目树/对话列表(列表类)按设计文档默认没有放大按钮，未实现，符合设计。
- **占位扫描**：无 TBD/TODO。"已知的一处简化"(文件列表/Web 共享同一份数据)、"已知的一处不一致"(右面板区宽度传参方式)、"背景虚化做不到只能变暗"三处都是刻意标注、给了具体原因和影响范围的记录，不是留白式占位。
- **类型/签名一致性**：`ShellState`/`ShellLayout`/`Divider`/`LeftView`/`RightView`/`MaximizedPane` 在 Task 1/3/4 之间的字段/变体名称通读过一遍，Task 4 往 `ShellLayout` 加字段时序与 Task 3 已用到的字段(`left_width`/`files_split` 等)没有冲突；`preview_pane`/`terminal_pane`/`project_pane` 的 `width: f32` 参数名在 Task 3/5 各处引用一致。
- **风险**：Task 3 是全计划里工程量、改动面最大的单一任务，本质是因为 Rust 编译器不允许"半改一半"的中间态——这不是任务拆分疏忏，是在诚实反映改动的耦合程度。真正执行时，如果 Step 15 的编译报错数量远超预期(比如超过 30 处)，值得停下来重新检查 Step 1-14 是否有遗漏的旧符号引用点，而不是逐条硬修到编译通过为止——大量报错通常意味着某个前置 Step 漏做了，不是"这个任务本来就该改这么多地方"。
