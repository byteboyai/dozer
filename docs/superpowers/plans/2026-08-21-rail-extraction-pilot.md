# Rail 代码抽取试点 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `crates/dozer-app/src/app.rs`(11,818 行)里图标栏(Rail)相关的
类型、纯逻辑、动画状态、渲染代码搬进新建的 `crates/dozer-app/src/rail.rs`,
不改变任何用户可见行为。这是 app.rs 巨石化拆分序列的第一个试点。

**Architecture:** `rail.rs` 是"类型 + 自由函数 + view"三件套模块(不是
`extensions/` 那种"私有 Message+State+update+view"的 extension 形态)——
`rail_layout`/`rail_drag`/`rail_slot_anims` 三个字段继续留在 `App`/
`ShellLayout` 上(多消费方共享数据,内核持有),`panel_select`/
`end_rail_drag` 的跨面板编排逻辑整体留在 `app.rs`,只把纯 Rail 逻辑
(消毒/拖拽计算/槽位动画/渲染)和它们的类型定义搬家。

**Tech Stack:** Rust, iced 0.14(`iced_widget`/`iced_winit`), `byteui` 主题库,
`serde`(`RailLayout` 持久化)。

**Spec:** `docs/superpowers/specs/2026-08-21-rail-extraction-pilot-design.md`

## Global Constraints

- **开发必须在独立分支上进行**(如 `feature/rail-extraction-pilot`),
  不得直接提交到 `main`;每个 Task 完成后提请审阅,全部 Task 审阅通过后
  才合并回 `main`。当前 `main` 上已有其他并发未提交改动(`git_log.rs`/
  `Cargo.toml`/`Cargo.lock`/`Info.plist`),开工前先用
  `superpowers:using-git-worktrees` 建一个隔离工作目录,不要在共享
  checkout 里直接切分支。
- **不改变任何用户可见行为**——这是纯代码组织重构,任何一步如果发现
  "顺手也把这段逻辑改一下会更好"的冲动,一律不做,记下来留到以后单独
  一轮。
- **不建 `Extension` trait / 运行时注册表,不拆独立 crate**——`rail.rs`
  就是 `crates/dozer-app/src/` 下的一个普通模块,同 `panel_layouts.rs`/
  `layout.rs` 量级。
- **每个 Task 结束时 `cargo build -p dozer-app` 与 `cargo test -p
  dozer-app` 必须全绿**——这是纯移动重构,没有"预期失败的测试"这个
  阶段,任何编译错误/测试失败都是这一步引入的回归,当场修好才能进入
  下一步,不得带着已知失败进入下一个 Task。
- **行号仅供定位参考,不是权威**——本计划里给出的行号是撰写时
  (`main` commit `fe7799b`)的快照,前面的 Task 一旦改动 `app.rs`,后面
  Task 里的行号就会漂移。每个 Task 开工前先用计划里给出的
  `grep -n "fn <name>"` / 文档注释首行等锚点字符串重新定位,不要死信
  行号。

---

## 文件结构总览

- **新建** `crates/dozer-app/src/rail.rs`——图标栏类型、纯逻辑、槽位动画、
  渲染,三个 Task 分批填入。
- **修改** `crates/dozer-app/src/main.rs`——加一行 `mod rail;`。
- **修改** `crates/dozer-app/src/app.rs`——删掉搬走的代码;`App`/
  `ShellLayout` 上三个字段的类型改成 `rail::` 限定;`panel_select`/
  `end_rail_drag`/`sanitize_shell_layout`/`panel_mirrored`/
  `rail_drag_move`/`dragging_rail`/`dragged_panel_kind`/
  `rail_drag_confirmed`/`advance_rail_slot_anims`/
  `any_rail_slot_anim_active`/`rail_slot_position` 这些**留在** `app.rs`
  的方法里,对已搬迁函数的调用改成 `rail::` 限定;`view()` 里三处
  view 函数调用点加 `rail::` 前缀;一个测试(`is_in_preview_column_
  returns_project_when_project_on_right`)里的 `RailLayout` 字面量加
  `rail::` 前缀。
- **修改** `crates/dozer-app/src/layout.rs`——`#[cfg(test)] mod tests`
  内 `use crate::app::RailLayout;` 改成 `use crate::rail::RailLayout;`。

---

## Task 1: 搬迁 Rail 类型定义 + 纯逻辑函数 + 23 个既有测试

**Files:**
- Create: `crates/dozer-app/src/rail.rs`
- Modify: `crates/dozer-app/src/main.rs`
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/layout.rs`
- Test: 既有测试原样随代码搬迁,断言不变(见 Step 6 清单)

**Interfaces:**
- Produces(后续 Task 2/3 依赖):
  - `pub enum rail::RailButton { Panel(PanelKind), HomeProjectList, HomeRecents, HomeBrowser }`
  - `pub struct rail::RailLayout { pub left: Vec<PanelKind>, pub right: Vec<PanelKind> }` + `impl RailLayout { pub fn side(&self, side: Side) -> &Vec<PanelKind>; pub fn side_mut(&mut self, side: Side) -> &mut Vec<PanelKind>; pub fn side_of(&self, kind: PanelKind) -> Side; }` + `impl Default for RailLayout`
  - `pub struct rail::RailDrag { pub source_side: Side, pub source_index: usize, pub origin_index: usize, pub pending_cross_side: Option<(Side, usize)>, pub press_pos: (f32, f32) }`(`#[derive(Debug, Clone, Copy, PartialEq)]`)
  - `pub(crate) struct rail::RailSlotAnim { current: f32, side: Side }`(字段私有,类型名 `pub(crate)`)+ `impl RailSlotAnim { fn retarget(&mut self, side: Side, target: f32); fn active(&self, target: f32) -> bool; }`
  - `pub(crate) fn rail::sanitize_rail_layout(rail: RailLayout) -> RailLayout`
  - `pub(crate) fn rail::rail_drag_move_into(rail: &mut RailLayout, drag: &mut RailDrag, side: Side, to: usize)`
  - `pub(crate) fn rail::rail_cross_apply(rail: &mut RailLayout, source_side: Side, source_index: usize, target_side: Side, target_index: usize) -> Option<PanelKind>`
  - `pub(crate) fn rail::dragged_panel_kind(rail: &RailLayout, drag: Option<RailDrag>) -> Option<PanelKind>`
  - `pub(crate) fn rail::rail_drag_past_threshold(drag: RailDrag, cursor: (f32, f32)) -> bool`
  - `pub(crate) fn rail::panel_mirrored_in(rail: &RailLayout, kind: PanelKind) -> bool`
- Consumes: `crate::app::{PanelKind, Side}`(两者都留在 `app.rs`,`rail.rs`
  只读取,不修改)。

### Step 1: 新建 `rail.rs`,注册模块

- [ ] 创建 `crates/dozer-app/src/rail.rs`,先写模块头注释和 `use`:

```rust
// crates/dozer-app/src/rail.rs
//! 图标栏(Rail):11 个面板挂载的两条侧栏,支持点击切换、同栏重排、跨栏
//! 拖拽换边。类型 + 纯逻辑 + 槽位动画 + 渲染都在这个模块——不是
//! `extensions/` 那种私有 Message+State+update+view 的 extension 形态,
//! `rail_layout`/`rail_drag`/`rail_slot_anims` 三个字段仍然挂在
//! `App`/`ShellLayout` 上(多消费方共享数据,内核持有),这里只是把纯
//! Rail 逻辑物理搬出 `app.rs`。见
//! `docs/superpowers/specs/2026-08-21-rail-extraction-pilot-design.md`。

use crate::app::{App, HoverId, Message, PanelKind, Side};
use serde::{Deserialize, Serialize};
```

  （后续 Step 4/5 若编译报缺少 `byteui`/`iced_widget` 等 import,按报错
  逐条补;这里先给出确定需要的四个。）

- [ ] 在 `crates/dozer-app/src/main.rs` 里,`mod project_meta;` 和
  `mod term_model;` 之间(按字母序插入,现有列表本身是排序的)加一行:

```rust
mod project_meta;
mod rail;
mod term_model;
```

- [ ] 提交(此时 `rail.rs` 内容还是空壳,不影响编译):

```bash
git add crates/dozer-app/src/rail.rs crates/dozer-app/src/main.rs
git commit -m "chore(dozer-app): 新建空壳 rail.rs 模块"
```

### Step 2: 搬迁常量 + `RailButton`

- [ ] 在 `app.rs` 里用 `grep -n "RAIL_DRAG_VISUAL_THRESHOLD_PX: f32"` 定位
  常量定义(撰写时在 `dragged_panel_kind` 自由函数和
  `rail_drag_past_threshold` 之间,约第 986 行),剪切下面这段到
  `rail.rs`(追加在 Step 1 的 `use` 之后):

```rust
/// 图标栏拖拽从"按下武装"到"视觉判定为一次真的拖拽"所需的最小位移
/// (像素,窗口逻辑坐标系,同 `App::last_cursor`)。低于这个距离只是武装
/// 态,不展示幽灵图标/源图标变淡/抓手光标——见 `RailDrag::press_pos` 字段
/// 文档解释的"快速单击也会先武装"问题。数值对齐常见桌面 OS 的点击/拖拽
/// 判定阈值。
const RAIL_DRAG_VISUAL_THRESHOLD_PX: f32 = 4.0;
```

- [ ] 用 `grep -n "pub enum RailButton"` 定位(撰写时约第 110 行),连同它
  前面的文档注释(从"/// 四个图标栏按钮的标识"那一行开始)一起剪切到
  `rail.rs`:

```rust
/// 四个图标栏按钮的标识,用于追踪 hover 态(图标颜色在 hover 时需变金,
/// 而 SVG 颜色在构建时就定死、不随 `button::Status` 变化,所以得在 App
/// 里记一个 hovered 目标,改色时按它重算)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RailButton {
    Panel(PanelKind),
    /// 首页左栏"项目列表" pane 图标。
    HomeProjectList,
    /// 首页左栏"Recents" pane 图标。
    HomeRecents,
    /// 首页右栏"浏览器" pane 图标。
    HomeBrowser,
}
```

- [ ] `app.rs` 里 `HoverId` 枚举内 `Rail(RailButton),` 那一行改成
  `Rail(rail::RailButton),`。

- [ ] `app.rs` 顶部 `use` 区块(`use crate::preview::WebviewSpec;` 那一行
  附近,按字母序)加一行 `use crate::rail;`。

### Step 3: 搬迁 `RailLayout`(含 `impl` 与 `Default`)——**注意
`panel_mirrored_in` 夹在 `impl RailLayout` 和 `impl Default for
RailLayout` 中间,一起搬,不要漏**

- [ ] 用 `grep -n "pub struct RailLayout"` 定位起点(撰写时约第 374 行,
  前面紧邻的文档注释"/// 每个面板当前挂在哪条图标栏..."一起搬),用
  `grep -n "impl Default for RailLayout"` 定位到它结束的闭合大括号(该
  `impl` 块只有一个 `fn default()`,数到匹配的 `}` 为止)。整段剪切到
  `rail.rs`:

```rust
/// 每个面板当前挂在哪条图标栏、栏内什么顺序——图标栏拖拽换栏(Stage 4)
/// 的唯一真相源。这个 Stage 只负责定义类型 + 持久化,渲染/交互还没有
/// 任何地方读它(那是 Stage 2/4 的范围),所以此刻它的值必然等于
/// `default()`,不会有别的取值出现。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RailLayout {
    pub left: Vec<PanelKind>,
    pub right: Vec<PanelKind>,
}

impl RailLayout {
    /// 返回某一侧图标栏当前挂载的面板列表(渲染顺序)。
    pub fn side(&self, side: Side) -> &Vec<PanelKind> {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    #[allow(dead_code)]
    pub fn side_mut(&mut self, side: Side) -> &mut Vec<PanelKind> {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    /// 给定面板,反查它当前挂在哪条栏。`RailLayout` 的不变式(见
    /// `sanitize_rail_layout`)保证 11 个面板不重不漏分布在两条栏,
    /// 所以这里的 `expect` 不会在合法状态下触发——`RailLayout` 一旦
    /// 通不过消毒就已经在 `layout::load_from` 里回落 `default()` 了,
    /// 不会带着"某个面板哪条栏都不在"的坏数据流到这里。
    pub fn side_of(&self, kind: PanelKind) -> Side {
        if self.left.contains(&kind) {
            Side::Left
        } else if self.right.contains(&kind) {
            Side::Right
        } else {
            unreachable!(
                "RailLayout 不变式被破坏:{kind:?} 不在任何一条栏——\
                 sanitize_rail_layout 应该已经挡掉这种坏数据"
            )
        }
    }
}

/// 面板当前是否偏离了默认栏(`side_of(kind) != default_side()`)。抽成
/// 自由函数单纯是为了让单元测试不必构造一个完整 `App`(它需要 `Client`/
/// 事件循环钩子)——逻辑本身和 `App::panel_mirrored` 完全一致,后者只是
/// 把自己的 `rail_layout` 喂进来。
pub(crate) fn panel_mirrored_in(rail: &RailLayout, kind: PanelKind) -> bool {
    rail.side_of(kind) != kind.default_side()
}

impl Default for RailLayout {
    fn default() -> Self {
        Self {
            left: vec![
                PanelKind::Project,
                PanelKind::Todo,
                PanelKind::Files,
                PanelKind::GitLog,
                PanelKind::Database,
                PanelKind::Ssh,
                PanelKind::Web,
            ],
            right: vec![
                PanelKind::Agent,
                PanelKind::Conversations,
                PanelKind::Usage,
                PanelKind::Acceptance,
            ],
        }
    }
}
```

  （`panel_mirrored_in` 原本是 `fn`,这里升级成 `pub(crate) fn`——
  `app.rs` 里 `App::panel_mirrored` 方法要跨模块调用它。）

- [ ] `app.rs` 里 `ShellLayout` 结构体的 `pub rail_layout: RailLayout,`
  字段类型改成 `pub rail_layout: rail::RailLayout,`。

- [ ] `impl Default for ShellLayout` 里 `rail_layout: RailLayout::default(),`
  改成 `rail_layout: rail::RailLayout::default(),`。

- [ ] `sanitize_shell_layout` 函数体里 `rail_layout: sanitize_rail_layout(l.rail_layout),`
  改成 `rail_layout: rail::sanitize_rail_layout(l.rail_layout),`(此时
  `sanitize_rail_layout` 还没搬,先留着这个前缀,等 Step 4 搬完它这行才
  真正生效——中间状态编译会报"找不到 `rail::sanitize_rail_layout`",属于
  预期的临时状态,继续做完 Step 4 再验证编译;真正的编译验证在 Step 7
  统一做,Step 3/4/5/6 之间不需要逐步编译)。

- [ ] `App::panel_mirrored` 方法体 `panel_mirrored_in(&self.shell_layout.rail_layout, kind)`
  改成 `rail::panel_mirrored_in(&self.shell_layout.rail_layout, kind)`。

- [ ] 用 `grep -n "fn is_in_preview_column_returns_project_when_project_on_right"`
  定位这个测试(留在 `app.rs`,不搬),把测试体里
  `rail_layout: RailLayout {` 这一行改成 `rail_layout: rail::RailLayout {`。

### Step 4: 搬迁剩余 4 个纯函数(`sanitize_rail_layout`/
`rail_drag_move_into`/`rail_cross_apply`/`dragged_panel_kind`/
`rail_drag_past_threshold`)

- [ ] 用 `grep -n "fn sanitize_rail_layout"` 定位,连同前面文档注释一起
  剪切到 `rail.rs`(接在 Step 3 内容后面):

```rust
/// `RailLayout` 的消毒:任一栏为空,或两侧合计不是恰 11 个不重复的
/// `PanelKind`(手改/版本不一致导致的坏数据),整个回落 `default()`。
/// 不做部分修复——缺一个面板就补在默认栏这种中间态比"直接用默认值"
/// 更难排查。
pub(crate) fn sanitize_rail_layout(rail: RailLayout) -> RailLayout {
    if rail.left.is_empty() || rail.right.is_empty() {
        return RailLayout::default();
    }
    let mut all: Vec<_> = rail.left.iter().chain(rail.right.iter()).collect();
    all.sort_by_key(|k| format!("{k:?}"));
    all.dedup();
    if all.len() != 11 || rail.left.len() + rail.right.len() != 11 {
        return RailLayout::default();
    }
    rail
}
```

  （原来是 `fn`,升级成 `pub(crate) fn`——`app.rs` 的
  `sanitize_shell_layout` 要跨模块调用。）

- [ ] 用 `grep -n "fn rail_drag_move_into"` 定位,连同前面文档注释剪切:

```rust
/// 图标栏拖拽"移动到 `side` 栏第 `to` 位"的纯逻辑核心:不依赖 `App` 的
/// 其他字段,抽成自由函数以便单元测试直接构造 `RailLayout` 验证(同 Stage 3
/// `panel_mirrored_in` 的做法的理由——`App` 需要 `Client`/事件循环钩子,
/// 构造成本高)。
///
/// 同栏(*`drag.source_side == side`*):立即重排(`Vec::remove`+`insert`),
/// 并把 `drag.source_index` 更新为新的源位置、清掉任何跨栏悬停残留。跨栏:
/// 只记 `drag.pending_cross_side`,具体搬移留给 `rail_cross_apply`/`App::
/// `end_rail_drag` 统一提交——避免每帧 `CursorMoved` 都触发一次 `Vec` 搬移
/// 和后续的布局存盘。
pub(crate) fn rail_drag_move_into(rail: &mut RailLayout, drag: &mut RailDrag, side: Side, to: usize) {
    if drag.source_side == side {
        // 光标回到源栏:不再悬停另一栏,先取消可能残留的跨栏悬停目标,
        // 再做同栏内重排。语义上"悬停回源栏"就撤销了"将要跨栏"的意图。
        drag.pending_cross_side = None;
        let panels = rail.side_mut(side);
        let from = drag.source_index;
        if from == to || from >= panels.len() || to >= panels.len() {
            return;
        }
        let kind = panels.remove(from);
        panels.insert(to, kind);
        drag.source_index = to;
    } else {
        drag.pending_cross_side = Some((side, to));
    }
}
```

- [ ] 紧接着,用 `grep -n "fn rail_cross_apply"` 定位,连同前面文档注释
  剪切:

```rust
/// 跨栏移动的"真正落地"纯逻辑:把 `source_side` 第 `source_index` 个面板
/// 搬到 `target_side` 第 `target_index` 位,返回被移动的面板;源栏只剩这一个
/// 时(搬走会清空,不支持"栏清空",见 spec 非目标)或源下标越界时返回
/// `None`、`rail` 不被改动。目标下标越界时 clamp 到末尾。
pub(crate) fn rail_cross_apply(
    rail: &mut RailLayout,
    source_side: Side,
    source_index: usize,
    target_side: Side,
    target_index: usize,
) -> Option<PanelKind> {
    let source_panels = rail.side(source_side);
    if source_side == target_side || source_panels.len() <= 1 || source_index >= source_panels.len()
    {
        return None;
    }
    let kind = rail.side_mut(source_side).remove(source_index);
    let target_index = target_index.min(rail.side(target_side).len());
    rail.side_mut(target_side).insert(target_index, kind);
    Some(kind)
}
```

- [ ] 紧接着,用 `grep -n "^fn dragged_panel_kind"` 定位(注意跟
  `impl App { pub fn dragged_panel_kind` 方法区分,搬的是自由函数版本),
  连同前面文档注释剪切:

```rust
/// 当前正被图标栏拖拽的面板种类(`None` = 未在拖拽)。纯查询,不修改
/// `rail`/`drag`——拖拽中同栏重排会实时更新 `drag.source_index`(见
/// `rail_drag_move_into`),所以这里查到的永远是"此刻鼠标下真正拖着的
/// 那个图标",不是拖拽开始时的原始位置。下标越界(理论不会发生,防御性)
/// 时返回 `None`,不 panic。
pub(crate) fn dragged_panel_kind(rail: &RailLayout, drag: Option<RailDrag>) -> Option<PanelKind> {
    let drag = drag?;
    rail.side(drag.source_side).get(drag.source_index).copied()
}
```

- [ ] 紧接着,用 `grep -n "fn rail_drag_past_threshold"` 定位,连同前面
  文档注释剪切:

```rust
/// `drag` 从武装(按下)到 `cursor`(当前 `App::last_cursor`)是否已经
/// 越过 [`RAIL_DRAG_VISUAL_THRESHOLD_PX`]——越过才算"确认是一次拖拽,不是
/// 单击",视图层据此决定要不要展示拖拽视觉。纯查询,不修改 `drag`。
pub(crate) fn rail_drag_past_threshold(drag: RailDrag, cursor: (f32, f32)) -> bool {
    let dx = cursor.0 - drag.press_pos.0;
    let dy = cursor.1 - drag.press_pos.1;
    dx * dx + dy * dy > RAIL_DRAG_VISUAL_THRESHOLD_PX * RAIL_DRAG_VISUAL_THRESHOLD_PX
}
```

  （原来都是 `fn`,四个统一升级成 `pub(crate) fn`——`app.rs` 里留下的
  `App` 方法要跨模块调用它们。）

### Step 5: 搬迁 `RailDrag`

- [ ] 用 `grep -n "pub struct RailDrag"` 定位,连同前面文档注释("/// 正在
  进行的图标栏面板拖拽..."那一行开始)剪切到 `rail.rs`(建议放在
  `RailButton` 之后、`RailLayout` 之前,跟原文件里"类型先于纯函数"的
  顺序一致,顺序本身不影响编译,但保持可读性):

```rust
/// 正在进行的图标栏面板拖拽(同栏重排 / 跨栏移动)。语义、生命周期管理
/// 手法照抄 `TabDrag`,但不复用它——`TabDrag`/`TabGroup` 是"同组内换位",
/// 图标栏这次还要支持"跨栏移动",合并进同一个类型会让校验逻辑变复杂。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RailDrag {
    pub source_side: Side,
    pub source_index: usize,
    /// 拖拽开始时 `source_index` 的原始值,不在拖拽期间随同栏重排更新。
    /// `end_rail_drag` 用它判断纯同栏重排是否真的发生过(优先级重排会
    /// 推高 `source_index`,未发生则保持原值),决定要不要把新顺序落盘。
    pub origin_index: usize,
    /// 悬停到另一栏时记录目标位置;`RailDragEnd` 才真正提交搬移,悬停
    /// 期间不搬、不落盘。悬停回源栏(或还没悬停到任何另一栏位置)时是
    /// `None`。
    pub pending_cross_side: Option<(Side, usize)>,
    /// 按下瞬间的 `App::last_cursor`,拖拽期间不更新——`rail_drag_confirmed`
    /// 用它和当前光标算位移,判断这是不是"真的在拖"(见其定义)。之所以
    /// 不能靠 `dragging_rail()`(`rail_drag.is_some()`)本身:那个从按下
    /// 瞬间就为真(见 `rail_drag_surface` 文档解释的"按下即武装"原因),
    /// 快速单击(按下几乎立刻松开、光标几乎不动)也会先武装再清空,若视觉
    /// (幽灵图标/源图标变淡/抓手光标)直接跟 `dragging_rail()` 走,会在
    /// 单击时闪一下"进入拖拽"的效果。
    pub press_pos: (f32, f32),
}
```

- [ ] `app.rs` 里 `App` 结构体字段 `rail_drag: Option<RailDrag>,` 改成
  `rail_drag: Option<rail::RailDrag>,`。

- [ ] `App::new_shell`(或对应构造函数)里字段初始化 `rail_drag: None,`
  不需要改(`None` 不携带类型名)。

- [ ] `App::panel_select` 方法体里构造 `self.rail_drag = Some(RailDrag {`
  这一行改成 `self.rail_drag = Some(rail::RailDrag {`。

### Step 6: 搬迁 `RailSlotAnim` 类型(逻辑本身留到 Task 2,这里只搬类型
定义,升级可见性)

- [ ] 用 `grep -n "struct RailSlotAnim"` 定位,连同前面文档注释("/// 图标
  栏按钮的动画槽位状态机..."那一行开始)剪切到 `rail.rs`:

```rust
/// 图标栏按钮的动画槽位状态机:`current` 是本帧渲染用的浮点槽位号(在
/// `rail_layout` 里的下标,逼近 `target` 中),`side` 记录上一次逼近所在的
/// 栏——同栏内 `target` 变化(重排让位)时正常指数逼近,平滑滑动;`side`
/// 本身变化(跨栏移动)时说明这是两条完全不同的物理列,`current`/`target`
/// 数值上的"接近"没有几何意义,`retarget` 直接 snap 到新 `target`,不生成
/// 滑动动画。逼近手法与 `HoverAnim` 同源,只是目标值域从"0..=1 悬停进度"
/// 换成"任意非负槽位号"。
#[derive(Debug, Clone, Copy)]
pub(crate) struct RailSlotAnim {
    current: f32,
    side: Side,
}

impl RailSlotAnim {
    /// 把这个按钮的目标槽位设成 `(side, target)`,必要时朝它逼近一拍。
    /// `side` 与上次不同(跨栏移动)时直接 snap,不留一帧"跨列插值"的
    /// 视觉噪音。
    pub(crate) fn retarget(&mut self, side: Side, target: f32) {
        if self.side != side {
            self.side = side;
            self.current = target;
            return;
        }
        let next = self.current + (target - self.current) * 0.5;
        self.current = if (next - target).abs() < 0.02 {
            target
        } else {
            next
        };
    }
    /// 动画是否仍在进行中(某按钮的槽位还没收敛到目标)。调用方需要先把
    /// `target` 通过 `retarget` 写入才能得到有意义的结果——这个方法只读
    /// 当前状态,不推进。
    pub(crate) fn active(&self, target: f32) -> bool {
        (self.current - target).abs() > 0.005
    }
}
```

  （原来 `struct`/`fn` 都是私有,这里 `struct` 升级 `pub(crate)`(字段
  本身保持私有,`app.rs` 只需要按类型名声明字段,不需要读写字段);
  `retarget`/`active` 两个方法升级 `pub(crate)`——Task 2 里 `app.rs`
  的薄包装方法要跨模块调用它们。）

- [ ] `app.rs` 里 `App` 结构体字段
  `rail_slot_anims: std::collections::HashMap<PanelKind, RailSlotAnim>,`
  改成 `rail_slot_anims: std::collections::HashMap<PanelKind, rail::RailSlotAnim>,`。

### Step 7: 编译并修补 import

- [ ] 运行 `cargo build -p dozer-app 2>&1 | head -100`,把编译器报的
  "cannot find type/value/function `X` in this scope" 逐条修:通常是
  `rail.rs` 缺 `use`(补 `byteui`/`iced_widget` 等)或 `app.rs` 里还有
  遗漏的 `RailButton`/`RailLayout`/`RailDrag`/`RailSlotAnim` 裸引用没
  加 `rail::` 前缀(用 `grep -n "RailButton\|RailLayout\|RailDrag\b\|RailSlotAnim"
  crates/dozer-app/src/app.rs` 全量核对一遍,排除注释里的提及)。
- [ ] 反复修到 `cargo build -p dozer-app` 干净通过。

### Step 8: 搬迁 23 个既有测试

在 `rail.rs` 末尾新增:

```rust
#[cfg(test)]
mod tests {
    use super::*;
```

- [ ] 用 `grep -n "fn default_side_matches_rail_layout_default"` 起,到
  `fn sanitize_rail_layout_falls_back_to_default_on_bad_data` 那个测试
  函数结束(闭合大括号),把这一整段 **7 个平铺测试**(`#[test]` + 函数体,
  中间不要漏任何一个)剪切进上面的 `mod tests`:
  - `default_side_matches_rail_layout_default`
  - `panel_mirrored_false_when_rail_layout_is_default`
  - `panel_mirrored_true_when_manually_relocated`
  - `rail_layout_default_covers_all_panels_without_duplicates`
  - `rail_layout_side_accessors_map_correctly`
  - `side_of_finds_every_default_panel`
  - `sanitize_rail_layout_falls_back_to_default_on_bad_data`

  **不要搬** `zone_at_x_icon_rail_returns_none`——它在文件里更靠前(约
  第 10839 行,跟这 7 个测试之间隔着 `zone_at_x_*`/`clamp_left_width_*`/
  `clamp_files_split_*`/`split_drag_skips_update_on_zero_zone_width`/
  `right_pair_split_*`/`readme_*` 等一大段不相关测试,不是紧邻),测的是
  `zone_at_x`(留在 `app.rs`),函数名带 `icon_rail` 字样容易被搜索命中
  但不是这次要搬的对象。

- [ ] 用 `grep -n "mod rail_drag_tests"` 定位这整个子模块(到它的闭合
  `}`,含 `use super::*;`、`drag_at` 辅助函数、以及全部 12 个
  `#[test]`:`same_side_reorder_moves_kind`/
  `same_side_reorder_to_same_index_is_noop`/`cross_side_move_relocates_panel`/
  `cross_side_move_is_noop_when_source_side_would_become_empty`/
  `cross_side_move_with_stale_source_index_is_noop`/
  `hovering_back_to_source_side_cancels_the_pending_cross_move`/
  `dragged_panel_kind_none_when_not_dragging`/
  `dragged_panel_kind_reads_source_slot`/
  `dragged_panel_kind_none_when_index_out_of_bounds`/
  `past_threshold_false_when_cursor_has_not_moved`/
  `past_threshold_false_when_exactly_at_threshold`/
  `past_threshold_true_once_cursor_moves_past_it`),整段剪切进
  `rail.rs` 的 `mod tests`(作为 `mod rail_drag_tests` 嵌套子模块,原样
  保留嵌套结构,不要拍平)。

- [ ] 用 `grep -n "mod rail_slot_anim_tests"` 定位这整个子模块(到它的
  闭合 `}`,含全部 4 个 `#[test]`:
  `retarget_same_side_eases_toward_target_without_snapping_immediately`/
  `retarget_same_side_converges_and_snaps_after_enough_ticks`/
  `retarget_side_change_snaps_immediately_no_interpolation`/
  `retarget_target_unchanged_stays_converged`),整段剪切进 `rail.rs` 的
  `mod tests`。

- [ ] **不要搬**紧接在 `rail_slot_anim_tests` 后面的
  `apply_column_drag_ssh_todo_gitlog_mirror_tests`/
  `apply_column_drag_files_project_web_mirror_tests`/
  `apply_column_drag_right_pair_mirror_tests` 三个 mod——它们测的是
  `apply_column_drag`(留在 `app.rs`),留在原地不动。

- [ ] 关闭 `rail.rs` 的 `mod tests { ... }` 大括号。

### Step 9: `layout.rs` 的测试 import 修一处

- [ ] `crates/dozer-app/src/layout.rs` 的 `#[cfg(test)] mod tests` 内,
  `use crate::app::RailLayout;` 改成 `use crate::rail::RailLayout;`
  (顶部 `use crate::app::{ShellLayout, sanitize_shell_layout};` 那一行
  不含 `RailLayout`,不用动)。

### Step 10: 全量验证 + 提交

- [ ] `cargo build -p dozer-app` 通过。
- [ ] `cargo test -p dozer-app` 全绿,确认输出里能看到 23 个搬迁测试的
  名字(`cargo test -p dozer-app -- --list | grep -c "rail\|dragged_panel_kind\|retarget_\|sanitize_rail_layout\|default_side_matches\|side_of_finds_every"`
  粗略核对数量不少于 23)。
- [ ] `cargo clippy -p dozer-app --all-targets` 无新增警告。
- [ ] `cargo fmt` 格式化。
- [ ] 提交:

```bash
git add crates/dozer-app/src/rail.rs crates/dozer-app/src/app.rs \
  crates/dozer-app/src/main.rs crates/dozer-app/src/layout.rs
git commit -m "refactor(dozer-app): 搬迁 Rail 类型与纯逻辑到 rail.rs"
```

---

## Task 2: 搬迁槽位动画自由函数,内核方法改薄包装

**Files:**
- Modify: `crates/dozer-app/src/rail.rs`
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: Task 1 产出的 `rail::RailLayout`/`rail::RailSlotAnim`。
- Produces:
  - `pub(crate) fn rail::advance_slot_anims(rail: &RailLayout, anims: &mut HashMap<PanelKind, RailSlotAnim>)`
  - `pub(crate) fn rail::any_slot_anim_active(rail: &RailLayout, anims: &HashMap<PanelKind, RailSlotAnim>) -> bool`
  - `pub(crate) fn rail::slot_position(anims: &HashMap<PanelKind, RailSlotAnim>, side: Side, kind: PanelKind, target_idx: usize) -> f32`
  - `App::rail_drag_move`/`App::dragged_panel_kind`/`App::rail_drag_confirmed` 三个既有方法签名不变,内部改调 `rail::` 版本(Task 3 的 view 代码会调用它们)。

### Step 1: 把三个 `impl App` 方法体搬成 `rail.rs` 自由函数

- [ ] 用 `grep -n "fn advance_rail_slot_anims"` 定位(约在 `App::
  advance_hover_anims` 附近),原方法体:

```rust
fn advance_rail_slot_anims(&mut self) {
    for side in [Side::Left, Side::Right] {
        for (idx, &kind) in self.shell_layout.rail_layout.side(side).iter().enumerate() {
            let target = idx as f32;
            self.rail_slot_anims
                .entry(kind)
                .or_insert(RailSlotAnim {
                    current: target,
                    side,
                })
                .retarget(side, target);
        }
    }
}
```

  在 `rail.rs` 新增等价自由函数(`HashMap` 需要在 `rail.rs` 顶部加
  `use std::collections::HashMap;`):

```rust
/// 推进图标栏按钮的槽位动画一拍——两侧各自按 `rail_layout` 当前顺序
/// 现算每个面板的目标槽位号,`RailSlotAnim::retarget` 朝它逼近。
pub(crate) fn advance_slot_anims(rail: &RailLayout, anims: &mut HashMap<PanelKind, RailSlotAnim>) {
    for side in [Side::Left, Side::Right] {
        for (idx, &kind) in rail.side(side).iter().enumerate() {
            let target = idx as f32;
            anims
                .entry(kind)
                .or_insert(RailSlotAnim {
                    current: target,
                    side,
                })
                .retarget(side, target);
        }
    }
}
```

  `app.rs` 里原方法体改成薄包装:

```rust
fn advance_rail_slot_anims(&mut self) {
    rail::advance_slot_anims(&self.shell_layout.rail_layout, &mut self.rail_slot_anims);
}
```

- [ ] 同样处理 `any_rail_slot_anim_active`。原方法体:

```rust
fn any_rail_slot_anim_active(&self) -> bool {
    [Side::Left, Side::Right].into_iter().any(|side| {
        self.shell_layout
            .rail_layout
            .side(side)
            .iter()
            .enumerate()
            .any(|(idx, &kind)| {
                self.rail_slot_anims
                    .get(&kind)
                    .is_some_and(|a| a.side == side && a.active(idx as f32))
            })
    })
}
```

  `rail.rs` 新增:

```rust
/// 是否还有图标栏按钮的槽位动画在进行中。
pub(crate) fn any_slot_anim_active(rail: &RailLayout, anims: &HashMap<PanelKind, RailSlotAnim>) -> bool {
    [Side::Left, Side::Right].into_iter().any(|side| {
        rail.side(side)
            .iter()
            .enumerate()
            .any(|(idx, &kind)| {
                anims
                    .get(&kind)
                    .is_some_and(|a| a.side == side && a.active(idx as f32))
            })
    })
}
```

  `app.rs` 薄包装:

```rust
fn any_rail_slot_anim_active(&self) -> bool {
    rail::any_slot_anim_active(&self.shell_layout.rail_layout, &self.rail_slot_anims)
}
```

  （`a.side` 是 `RailSlotAnim` 的私有字段——`any_slot_anim_active` 定义
  在 `rail.rs` 同一个模块内,能访问私有字段,不受 `pub(crate) struct`
  只公开类型名、字段仍私有的限制。）

- [ ] 同样处理 `rail_slot_position`。原方法体:

```rust
fn rail_slot_position(&self, side: Side, kind: PanelKind, target_idx: usize) -> f32 {
    let target = target_idx as f32;
    self.rail_slot_anims
        .get(&kind)
        .filter(|a| a.side == side)
        .map(|a| a.current)
        .unwrap_or(target)
}
```

  `rail.rs` 新增:

```rust
/// `kind` 在 `side` 栏当前应渲染的动画槽位号(浮点,逼近中的
/// `rail_layout` 下标)。渲染层据此算按钮的 y 偏移,取代直接用
/// `rail_layout` 下标瞬间跳变。
pub(crate) fn slot_position(anims: &HashMap<PanelKind, RailSlotAnim>, side: Side, kind: PanelKind, target_idx: usize) -> f32 {
    let target = target_idx as f32;
    anims
        .get(&kind)
        .filter(|a| a.side == side)
        .map(|a| a.current)
        .unwrap_or(target)
}
```

  `app.rs` 薄包装:

```rust
fn rail_slot_position(&self, side: Side, kind: PanelKind, target_idx: usize) -> f32 {
    rail::slot_position(&self.rail_slot_anims, side, kind, target_idx)
}
```

### Step 2: 已是薄包装的方法改调 `rail::` 版本

- [ ] `App::rail_drag_move`:

```rust
fn rail_drag_move(&mut self, side: Side, to: usize) {
    let Some(mut drag) = self.rail_drag else {
        return;
    };
    rail::rail_drag_move_into(&mut self.shell_layout.rail_layout, &mut drag, side, to);
    self.rail_drag = Some(drag);
}
```

- [ ] `App::dragged_panel_kind`:

```rust
pub fn dragged_panel_kind(&self) -> Option<PanelKind> {
    rail::dragged_panel_kind(&self.shell_layout.rail_layout, self.rail_drag)
}
```

- [ ] `App::rail_drag_confirmed`:

```rust
pub fn rail_drag_confirmed(&self) -> bool {
    self.rail_drag
        .is_some_and(|d| rail::rail_drag_past_threshold(d, self.last_cursor))
}
```

- [ ] `App::end_rail_drag` 方法体内部调用 `rail_cross_apply(` 那一处改成
  `rail::rail_cross_apply(`,方法其余部分(切换 `left_view`/`right_view`/
  `left_collapsed`/`right_collapsed`、调 `on_shell_layout_changed()`)
  原样不动。

- [ ] `App::dragging_rail` 不引用任何搬迁类型/函数,原样不动。

### Step 3: 验证 + 提交

- [ ] `cargo build -p dozer-app` 通过。
- [ ] `cargo test -p dozer-app` 全绿。
- [ ] `cargo clippy -p dozer-app --all-targets` 无新增警告。
- [ ] `cargo fmt`。
- [ ] 提交:

```bash
git add crates/dozer-app/src/rail.rs crates/dozer-app/src/app.rs
git commit -m "refactor(dozer-app): 图标栏槽位动画搬迁到 rail.rs,内核方法改薄包装"
```

---

## Task 3: 搬迁 Rail 渲染函数

**Files:**
- Modify: `crates/dozer-app/src/rail.rs`
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: Task 1/2 产出的全部 `rail::` 类型与函数;`crate::app::App`
  (只读引用,view 函数不修改 `App`)。
- Produces:
  - `pub(crate) fn rail::rail_icon_button<'a>(icon: icons::IconKind, active: bool, hover_t: f32, msg: Message, tooltip: &'a str) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>`
  - `pub(crate) fn rail::icon_rail(app: &App, side: Side) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>`
  - `pub(crate) fn rail::rail_drag_ghost(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>`

这个 Task 搬三块**不改一个字符**的现有代码,内容本身在计划里不重复
粘贴(几百行 iced 部件构造样板,复制粘贴即可,不是重新编写)——用下面
给出的 grep 锚点在 `app.rs` 里精确定界,连同各自前面的文档注释一起整块
剪切到 `rail.rs`(建议顺序:先 `rail_icon_button`,再 `icon_rail`+
`panel_meta`+`panel_badge`,再 `rail_drag_surface`,最后 `rail_drag_ghost`,
与原文件顺序一致)：

### Step 1: 剪切 `rail_icon_button` → `panel_meta` → `panel_badge`(含
`icon_rail`,四者物理相邻,一次性整块剪切)

- [ ] 起点:`grep -n "pub(crate) fn rail_icon_button"` 命中行往上数到
  紧邻的文档注释首行("/// 单个图标栏按钮：圆角正方形背景常驻...")。
- [ ] 终点:`grep -n "pub(crate) fn zone_pane_border"` 命中行的**前一行**
  (即 `panel_badge` 函数体的闭合 `}`)。
- [ ] 中间依次包含且顺序不变:`rail_icon_button`(`pub(crate) fn`)→
  `icon_rail`(`fn`,doc 注释里提到"图标栏:按 `app.shell_layout.rail_layout.side(side)`
  的顺序遍历渲染")→ `panel_meta`(`fn panel_meta(kind: PanelKind) ->
  (icons::IconKind, &'static str)`)→ `panel_badge`(`fn panel_badge(app:
  &App, kind: PanelKind) -> ...`)。**确认没有把 `PaneCorner`
  枚举定义带进来**(那是下一块,不属于 Rail)。
- [ ] 整块剪切到 `rail.rs`,函数体一字不改;但 `icon_rail` 与
  `rail_icon_button` 当前是模块私有 `fn`——搬到 `rail.rs` 后
  `app.rs::view()` 要跨模块调用 `icon_rail`,把它的签名从 `fn icon_rail`
  改成 `pub(crate) fn icon_rail`(`rail_icon_button` 已经是 `pub(crate)`,
  不用改;`panel_meta`/`panel_badge` 只在同模块内部被调用,保持私有
  `fn` 不变)。

### Step 2: 剪切 `rail_drag_surface`

- [ ] 起点:`grep -n "fn rail_drag_surface"` 命中行往上数到紧邻文档注释
  首行("/// 重排要求光标移到另一个按钮上...")。
- [ ] 终点:`grep -n "pub(crate) fn panel_tab"` 命中行的前一行。
- [ ] **确认没有把它前面的 `tab_drag_surface`(`pub(crate) fn
  tab_drag_surface`,通用页签拖拽,不是 Rail)带进来**——`rail_drag_surface`
  自己才是要搬的。
- [ ] 整块剪切到 `rail.rs`,函数体一字不改,保持模块私有 `fn`(只被同
  模块的 `icon_rail` 调用,不需要 `pub(crate)`)。

### Step 3: 剪切 `rail_drag_ghost`

- [ ] 起点:`grep -n "fn rail_drag_ghost"` 命中行往上数到紧邻文档注释
  首行("/// 图标栏拖拽换栏时跟随光标的幽灵图标..."附近,具体首行以
  实际内容为准)。
- [ ] 终点:`grep -n "fn ssh_empty_state"` 命中行的前一行。
- [ ] 整块剪切到 `rail.rs`,函数体一字不改;签名从 `fn rail_drag_ghost`
  改成 `pub(crate) fn rail_drag_ghost`(`app.rs::view()` 要跨模块调用)。

### Step 4: 修 `app.rs::view()` 里的三处调用点

- [ ] `body = row![` 里 `icon_rail(self, Side::Left),` 改成
  `rail::icon_rail(self, Side::Left),`。
- [ ] 同一个 `row!` 里 `icon_rail(self, Side::Right),` 改成
  `rail::icon_rail(self, Side::Right),`。
- [ ] `if self.rail_drag_confirmed() { stack![with_maximize,
  rail_drag_ghost(self)]` 里 `rail_drag_ghost(self)` 改成
  `rail::rail_drag_ghost(self)`。

### Step 5: 编译修补 + 验证 + 提交

- [ ] `cargo build -p dozer-app 2>&1 | head -100`,按报错逐条补
  `rail.rs` 缺的 `use`(预期需要 `byteui::theme::{color, geometry,
  icon_size}`、`byteui::interaction::icons`、
  `iced_widget::{button, column, container, row, stack, text, MouseArea}`、
  `iced_widget::core::{Border, Color, Element, Length, Padding, mouse}`、
  `iced_widget::tooltip` 等——具体以编译器报的缺失符号为准,不要预先
  猜全,按报错补全即可)。同时核对 `app.rs` 里是否还有遗漏的裸
  `icon_rail(`/`rail_drag_ghost(`/`rail_icon_button(`/`rail_drag_surface(`
  调用没加前缀(`grep -n "icon_rail(\|rail_drag_ghost(\|rail_icon_button(\|rail_drag_surface("
  crates/dozer-app/src/app.rs` 核对,排除 `rail.rs` 内部的调用——那些
  本来就不需要前缀)。
- [ ] 反复修到 `cargo build -p dozer-app` 干净通过。
- [ ] `cargo test -p dozer-app` 全绿(这个 Task 不改逻辑,不新增测试,
  只是确认现有测试没被搬动过程中的误删/误改波及)。
- [ ] `cargo clippy -p dozer-app --all-targets` 无新增警告。
- [ ] `cargo fmt`。
- [ ] 提交:

```bash
git add crates/dozer-app/src/rail.rs crates/dozer-app/src/app.rs
git commit -m "refactor(dozer-app): 图标栏渲染函数搬迁到 rail.rs"
```

---

## Task 4: 全工作区验证 + 人工 GUI 验收

**Files:** 无代码改动,仅验证。

- [ ] `cargo build`(全 workspace,不加 `-p`,确认没有其他 crate 依赖
  `dozer-app` 内部路径受影响——按 CLAUDE.md 已知 `dozer-app` 是叶子 bin
  crate,预期无影响,但仍要跑一遍全量构建兜底)。
- [ ] `cargo test`(全 workspace)。
- [ ] `cargo clippy --all-targets`(全 workspace)。
- [ ] `cargo fmt --check`(全 workspace,确认没有遗漏没格式化的文件)。
- [ ] `wc -l crates/dozer-app/src/app.rs crates/dozer-app/src/rail.rs`,
  确认 `app.rs` 行数比 Task 1 开工前显著减少、`rail.rs` 落地了预期规模
  的代码(粗略核对,不要求精确到行)。
- [ ] `cargo run -p dozer-app` 启动 GUI,人工过一遍(对照 spec 的"测试
  策略"章节):
  - 图标栏点击切换左右两侧各个面板,展开/收起行为正常。
  - 按住某个图标栏按钮拖拽做同栏内重排,松手顺序生效且重启后
    (`layout.json`)顺序保留。
  - 拖拽跨栏(如把某个左栏面板拖到右栏),验证:
    - 移动后面板确实出现在目标栏且自动展开。
    - 源栏若原本 active 就是它,自动切到源栏剩余的第一个面板。
    - 三个挂了 wry webview 的面板(`Files`/`Project`/`Web`)搬到对侧
      后几何镜像正确(webview 没有错位/裁切)。
  - 快速单击(不拖动)不触发任何拖拽视觉(幽灵图标/抓手光标/源图标
    变淡)。
  - 图标栏按钮 hover 时颜色平滑过渡到金色,选中态图标带金色外框。
  - 拖拽换位时槽位滑动动画平滑(同栏内),跨栏落地时直接 snap(不平滑
    过渡)。
  - 重启应用,确认 `rail_layout` 从上次的拖拽结果正确读回,没有回落
    到默认布局。
- [ ] 全部通过后,把 Task 1-4 的分支提请代码审阅(`superpowers:
  requesting-code-review`),审阅通过后合并回 `main`。
