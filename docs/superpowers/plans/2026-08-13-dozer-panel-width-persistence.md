# 面板区宽度：12% 初始化 + 手动调整持久化 + 每项目独立配置 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 满足四项需求——
1. 两栏展开、聚焦 `left_zone`(即左面板区)时,左面板区初始宽度 = Dozer 窗口总宽的 12%；
2. 两栏展开、聚焦 `right_zone`(右面板区)时,右面板区初始宽度 = 窗口总宽的 12%；
3. 用户手动拖拽调整后的面板尺寸要落盘、重启后保留；
4. 每个项目各自能保存**不同**的面板尺寸配置,切项目时配套切换。

**结论先行（与甲方答复对齐）：**
- `life_zone` = `left_zone`(左面板区)。
- **右面板维持 Fill(吃掉左面板剩下的空间),不新增独立右宽字段**(答复 2 = "仍保持右为剩余")。因此需求 2 的"12%"在结构上只能落到"左面板收起、右面板独占 zones_width"这种场景——而那时右面板就是整个 `zones_width`(≈ 窗口宽 - 两条图标栏 - 分隔线,远大于 12%)。这需求实际被结构吞掉,**计划里显式记为显式未决项(需求 2 在"右为 Fill"前提下不产生额外字段)**,需要甲方点头或后续改回"右也存固定宽"再实现。
- **每项目尺寸配置 = 左宽 `left_width` + 四个 split** 组成(右宽是 Fill 导出值,无需存)。把这五个字段从全局 `ShellLayout` 迁进每项目存储(挂到现有 `panel_layouts.json` 的 `PanelLayout` 上),切项目时 `adopt_panel_layout`/`stash_active_panel_layout` 已存在的那套切换流程原样复用。

**明确三个语义(按甲方澄清,非冲突):**
1. **12% 只作初始默认**,不是"每次都要顶到 12%"的目标值;系统对左右面板在 zone 里的
   分配本就有内置默认,本改动只是**把这份默认的算法统一成一条规则(12% 窗口宽)**。
2. **用户手动调过的尺寸永远优先**:只要该项目已持久化过 `left_width`(存过),就用
   存档值,绝不重推导成 12%。12% 只在"该项目从没存过尺寸、需要算一个首次默认"时生效。
3. 初始默认仍走现有夹取链(`sanitize_*`/`clamp_left_width`):即 12% × 窗口宽算出
   的数值过一遍 `min_zone_width`(320px)下限。默认 1440×900 → 12% ≈ 172.8px,
   被垫到 **320px**,但这只是"首次默认",用户一拖就存成自己的值,**不产生"窄面板
   反而回不去"的问题**。要不要放开 320 下限允许 173px 窄面板,记为显式未决项,
   默认不放开。

**Architecture:** 当前模型(见 `app.rs` 顶部注释与 `panel_layouts.rs`):
- 全局几何 `App::shell_layout: ShellLayout`(`layout.json`),含 `left_width` + `files_split`/`project_split`/`agent_split`/`conversations_split` + `window_width/height`。左右 panel 宽度/分割比例**所有项目共享一份**。
- 每项目面板态 `App::panel_layouts: HashMap<i64, PanelLayout>`(`panel_layouts.json`),只记 `left_view`/`right_view`/`left_collapsed`/`right_collapsed`。切项目走 `stash_active_panel_layout`→`adopt_panel_layout`。

改造核心:**把五个尺寸字段(`left_width` + 四个 split)从 `ShellLayout` 挪进每项目的 `PanelLayout`**,让每项目各自存。`ShellLayout` 只留窗口尺寸(全局)。这样"每项目不同尺寸配置"直接复用现有逐项目切换机制,不引入第二套迁移。

**Tech Stack:** Rust workspace;iced 0.14;serde 已有;无新增外部依赖(`rfd` 已是既有依赖)。

**Spec:** 无独立 design spec(本需求原文即需求真相源)。几何/持久化权威注释见 `app.rs` 顶部及 `ShellLayout`/`PanelLayout`/`clamp_left_width`/`apply_column_drag`。

## Global Constraints

- **在独立分支上开发,不要直接提交到 main**:新建 `feature/panel-width-persistence`
  分支做全部改动,完成后提请审阅,审阅通过后再合并回 `main`。
- **核心不变量:12% 只是首次默认,永不覆盖手动调整。** 存在本项目任何已存档
  尺寸字段(`left_width > 0`)时,一律用存档值;`default_left_width_for` 只在
  "该项目从没存过尺寸"的首次默认时被调用一次。任何改动都不得让已持久化的手动
  宽度"回落/重推导"。
- 每个任务结束都要 `cargo build -p dozer-app && cargo test -p dozer-app &&
  cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过(已有 3 个
  与本次改动无关的已存在失败/警告豁免:`terminal_pane_pixel_size_right_maximized_matches_overlay_box`、
  `terminal_grid_state_sizes_hidden_terminal_as_if_shown` 两个测试、以及
  `extensions/files.rs`/`extensions/todo.rs` 的 clippy 警告)。
- `sanitize_shell_layout`/`clamp_left_width` 是**唯一**夹取点,渲染/几何/拖拽三侧必须共用,不得分散。
- `window_width`/`window_height` 保持全局,不迁进每项目(`layout.json` 语义不变)。
- 迁移要向后兼容:老 `layout.json` 里的 `left_width` 等字段作首开迁移来源;老 `panel_layouts.json` 缺尺寸字段时用全局值回填(见 Task 1)。
- 需求 2("右面板 12%")在"右为 Fill"下不新增字段,计划里保持为显式未决项,实现上不为其写任何右宽字段。

---

### Task 1: 把尺寸字段迁进每项目 `PanelLayout`,全局只留窗口尺寸

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/panel_layouts.rs`
- Modify: `crates/dozer-app/src/layout.rs`

**Interfaces:**
- `PanelLayout` 新增字段:`left_width: f32`、`files_split`、`project_split`、
  `agent_split`、`conversations_split`(后四个 `f32`)。`Default` 对应
  `ShellLayout::default()` 同名值。
- `ShellLayout` 删除 `left_width` 与四个 split 字段,只留 `window_width`/
  `window_height`(全局)。`ShellLayout::default()` 同步瘦身。
- `App::shell_layout` 语义改为"全局窗口尺寸";当前活跃项目的五个尺寸字段改
  由 `PanelLayout` 经现有 `adopt_panel_layout` 灌回活值(见 Task 2)。
- `sanitize_shell_layout` 退化为只夹窗口尺寸;夹取 split/left_width 的逻辑迁到
  `sanitize_panel_layout`(新函数,供 `panel_layouts::load_from` 调)。

- [ ] **Step 1: 改类型定义**
在 `app.rs` 给 `PanelLayout` 加 `left_width`/`files_split`/`project_split`/
`agent_split`/`conversations_split` 五个字段 + `#[serde(default)]`;`Default`
填 `ShellLayout::default()` 那组值(把该组数值常量提成 `fn default_panel_widths() -> (f32,f32,f32,f32,f32)`,两个 `Default` 共用,不各写各的)。

- [ ] **Step 2: ShellLayout 瘦身**
`ShellLayout` 删 `left_width` + 四个 split,只留 `window_width`/`window_height`。
`sanitize_shell_layout` 只夹窗口尺寸(现逻辑保留);新建 `sanitize_panel_layout(pl) -> PanelLayout`,把现在的 split 夹取 + left_width 夹下限逻辑整体搬过去。

- [ ] **Step 3: 持久化接线**
`layout.rs`：`load_from` 改调 `sanitize_shell_layout`(字段少了,天然兼容老文件);
`save_to` 不变。
`panel_layouts.rs`：`load_from` 读回 `HashMap<i64, PanelLayout>` 后逐项调
`sanitize_panel_layout`;**老 `panel_layouts.json` 缺尺寸字段时**(`#[serde(default)]`
补出 0.0)用全局 `layout::load()` 的值回填(把"上一版全局偏好"迁移进每个项目)。

- [ ] **Step 4: 单测**
补:老 `layout.json`(带 left_width/split)反序列化不炸、窗口尺寸保留、尺寸字段被忽略;
`sanitize_panel_layout` 夹 split/left_width 的下限(0.0 → MIN_SPLIT/MIN_ZONE);
`panel_layouts::load_from` 缺尺寸字段的老文件用全局值回填。

---

### Task 2: 每项目切换灌回/存下尺寸,`ShellState` 改从 PanelLayout 取宽

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- `App::current_panel_layout()` 把 `left_width`+四个 split 一起打包进
  `PanelLayout`(与 `stash_active_panel_layout` 拼接)。
- `App::adopt_panel_layout(id)` 把每项目的 `left_width`+split 灌回活值字段
  (新增若干 `self.*` 活值,或一个 `self.panel_dim: PanelDims` 小结构)。
- `ShellState.layout` 从 `ShellLayout`(只剩窗口尺寸)→ 需补 `panel_dim` 维度。
  方案:`ShellState` 保留 `layout: ShellLayout`(窗口尺寸),新增字段
  `dim: PanelDims`(五个尺寸);`apply_column_drag` 改读 `state.dim.left_width`
  /各 split,返回值同样改成 `PanelDims`(或沿用返回值里只带尺寸)。
- 渲染侧:`left_panel_area` 的 `files_split`/`project_split`、
  `right_panel_area` 的 `agent_split`/`conversations_split`、`Workspace::
  effective_left_width` 全部从 `self.shell_layout` 改读新的每项目活值。
- `App::shell_state()` 组装时把活值尺寸填进 `ShellState.dim`。

- [ ] **Step 1: 活值字段**
`App` 增每项目五个尺寸活值(直接 `left_width`/`files_split`/… 字段,对齐旧
`shell_layout` 的读法);`App::new` 初始化来自 `PanelLayout::default()`(或全局回填)。

- [ ] **Step 2: 切换流程**
`adopt_panel_layout` 灌尺寸;`current_panel_layout`/`stash_active_panel_layout`
存尺寸。`on_shell_layout_changed` 里 `spawn_shell_layout_save` 改为既存全局窗口
尺寸也 `spawn_panel_layouts_save`(每项目尺寸)。

- [ ] **Step 3: 几何/渲染改源**
`ShellState` 补 `dim`;`left_zone_width`/`right_zone_width`/`clamp_left_width`
调用点的 `left_width` 来源改每项目活值;`apply_column_drag` 各分支读/写
`dim`;`Workspace::effective_left_width`、`left_panel_area`、`right_panel_area`
的 split 读取全部改每项目活值。

- [ ] **Step 4: 单测**
`adopt`/`stash` 回环(项目 A 改宽 → 切 B → 切回 A 恢复);`apply_column_drag`
对每项目 `dim` 的读写;`ShellState` 组装。

---

### Task 3: 两栏展开时的 12% 初始宽度

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- 新增纯函数 `default_left_width_for(window_width: f32) -> f32`(含 12%;
  见"明确冲突"段,over 后再过 clamp)。
- 使用者:`PanelLayout::default()` 无法感知窗口宽 → 改为在
  `App::new`/`adopt_panel_layout` 遇"该项目从未存过尺寸"时,用
  `default_left_width_for(self.window_size.0)` 灌入 `left_width` 活值。

- [ ] **Step 1: 12% 初始值(仅默认)**
`default_left_width_for(window_width) -> f32` = `(0.12 * window_width).clamp(min_zone_width, 上界)`,
上界复用 `clamp_left_width` 的同款"给右面板留 min"规则。因 12% < 320,默认
窗口下实际返回 320 —— 在代码注释与下方"验收"里明示这点,不静默。这个函数
**只**在"首次默认"时被调用一次,**绝不在已有存档值时调用**。

- [ ] **Step 2: 触发条件 = 两栏展开 + 无存档尺寸"
两栏展开(`!(left_collapsed)`)时,若该项目从未持久化过尺寸(`PanelLayout` 的
`left_width` 为 0 / 该键第一次出现),用 `default_left_width_for(window_size.0)`
算首次默认灌入活值。**任何已存尺寸都直接生效,绝不覆盖**(12% 只在首次默认
时出现,见全局约束)。

- [ ] **Step 3: 需求 2 处理"
右面板维持 Fill,不写右宽字段。当左收起、右独占时,右面板 = `zones_width`
(非 12%)。右面板属于"由左面板推导的分配",按甲方澄清"系统本就对左右在 zone
里的分配有内置默认",右面板的默认沿用现 Fill 行为,不新增字段加注释锚点于
`right_zone_width` 处交代此决定。

- [ ] **Step 4: 单测"
`default_left_width_for(1440)=320`(验证 12% 被 min 夹到);宽窗口(如 8000)×12%
不被 clamp 掉 → `clamp_left_width` 上界内返回真实 12%;**已存档项目绝不触发
初始化覆盖**(存 500 → 切项目再回来仍 500,不回落 320/12%)。

---

### Task 4: 手动拖拽调宽落盘 + 回归验证

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- 拖拽 `Divider::LeftRight`/`LeftPairSplit`/`ProjectSplit`/`RightPairSplit` →
  `apply_column_drag` 已把结果写进每项目 `dim` 活值;任务只需保证
  `ColumnDragEnd`/`on_shell_layout_changed` 触发**每项目尺寸**落盘
  (`spawn_panel_layouts_save` 已覆盖)。全局窗口尺寸照旧。

- [ ] **Step 1: 落盘接线**
确认/补齐:拖拽结束 → `on_shell_layout_changed` → `spawn_panel_layouts_save`
(每项目) + `spawn_shell_layout_save`(全局窗口尺寸)。多页签共存时每项目独立存。

- [ ] **Step 2: 全量回归**
`cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app
--all-targets && cargo fmt`,修掉因字段迁移带来的编译/借用问题。

- [ ] **Step 3: 手动冒烟(记录在 plan 评审)**
两栏左区:项目 A 拖窄左面板 → 重开 app → 恢复窄宽;切到项目 B(首次)→ 左面板
= min(12% 被夹);切回 A → A 的窄宽还在。确认右面板始终 Fill。

---

### Task 5: 文档同步 + 提交

**Files:**
- Modify: `docs/superpowers/plans/2026-08-13-dozer-panel-width-persistence.md`
  (把"明确冲突/显式未决项"结论回填成最终定稿)

- [ ] **Step 1: 回填评审结论**
若评审拍板"允许 <320 的窄面板",额外一行记录:需调低 `min_zone_width` 或放开
左面板下限;若保持 320,记录"12% 在默认窗口下= 320"。

- [ ] **Step 2: 分支提交**
`feature/panel-width-persistence` 分支上分任务提交(Task 1-4 各自 commit),评审
通过后再提议合并 main。
