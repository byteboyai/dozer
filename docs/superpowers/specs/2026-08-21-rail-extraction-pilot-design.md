# Rail(图标栏)代码抽取试点设计

**状态:已批准(brainstorming 会话,2026-08-21)**

## 背景

`app.rs` 已涨到 11,819 行,核心是一个从 2565 行延伸到文件末尾的巨型
`impl App`(~9,254 行),其中 `view()` 单函数 2,905 行、`update()` 单函数
1,003 行。这正是 2026-08-20 那份 kooky 对比评价 triage 里标记为"最大架构
债务、需单独立项"而搁置的一项——当时用户选择先不立项,这次因维护成本已到
临界点重新捡起。

前 8 个面板扩展化试点(git_log/browser/todo/files/usage/acceptance/project/
database 等,`extensions/` 目录)都是"自己的 Message+私有 State+update+view
自洽整体,内核只留一个包装变体转发"的形态,且都是**可选侧边面板**。这次要
处理的是 `view()`/`update()` 膨胀里跟"核心面板尚未抽取"绑在一起的部分——
按风险从低到高排序为 Rail(图标栏拖拽)→ Terminal → Preview,Topbar/项目
页签栏因为本质是 App 自己的多项目管理 UI(非"可选面板"),留到以后按纯 view
拆函数处理,不进这个序列。这次只做 **Rail**。

Rail 相关代码(`RailButton`/`RailLayout`/`RailDrag`/`RailSlotAnim` 及其
拖拽/动画/渲染逻辑)最初由
[图标栏面板拖拽换栏设计](2026-08-19-rail-panel-drag-relocation-design.md)
引入,当时直接写进了 `app.rs`。这次是纯代码组织重构,不改变该功能的任何
现有行为。

摸底发现 `rail_layout: RailLayout` 挂在 `ShellLayout`(App 全局,不是按
项目),被 `app.rs` 里 15+ 处 `side_of()` 调用点读取(geometry 计算、
`view()` 里各面板判断自己该渲染在哪一侧、mirrored 徽标判断),不是 Rail
私有状态;`RailButton` 也只是共享 `HoverId` 枚举的一个 variant,悬停动画
机制(`hover_anims`/`set_hover`/`hover_progress`)与 `TopbarButton`/
`ProjectTabItem`/`TermTabItem` 等共用。这意味着 Rail **不适用**前 8 个
试点"私有 State"的形态——硬套会导致要么把 `rail_layout` 私有化后到处加
accessor(为 15 个读者重新发明一层间接),要么表面私有实际还是到处 `pub`,
两者都不比现状干净。

进一步摸底发现 `panel_select`(点图标栏触发面板切换)和 `end_rail_drag`
(松手结束拖拽)虽然都会touch `self.rail_drag`/`rail_layout`,但同时也在
做跨面板编排——切换 `left_view`/`right_view`/`left_collapsed`/
`right_collapsed`、触发各面板"切入时动作"(`sync_git_log_to_active_project`/
`todo::reload_from_disk`/`database::reload_from_disk`/
`ensure_project_readme_and_reveal`/`ssh::reload_from_disk`/
`spawn_usage_refresh`/打开验收 tab)、清 `maximized`、调
`on_shell_layout_changed()`。这部分是内核编排职责(呼应 Project 试点定下的
"多消费方数据编排权收归内核"原则),不是 Rail 私有逻辑,**留在 `app.rs`**。

## 目标 / 非目标

**目标**:

1. 新建 `crates/dozer-app/src/rail.rs`(**不进 `extensions/` 目录**——它
   没有私有 `WorkspaceState`/`AppState`,是"类型 + 纯函数 + view"三件套,
   跟 `panel_layouts.rs`/`layout.rs` 同一量级的模块,不是"自洽 Message+
   State+update+view"的 extension 形态)。
2. 纯函数原样搬入(签名不变,只是物理位置移动):
   `sanitize_rail_layout`、`rail_drag_move_into`、`rail_cross_apply`、
   `dragged_panel_kind`(自由函数版本)、`rail_drag_past_threshold`、
   `panel_mirrored_in`。
3. 类型定义搬入:`RailButton`、`RailLayout`(含其 `impl` 块:`side`/
   `side_mut`/`side_of`)、`RailDrag`、`RailSlotAnim`(含其 `impl` 块:
   `retarget`/`active`)。`HoverAnim`/`HoverId` **不搬**——两者是跨 Rail/
   Topbar/项目页签/终端页签共用的基础设施,不是 Rail 私有类型。
4. 三个不带跨面板编排逻辑的 `impl App` 方法**转成 `rail.rs` 里接收显式
   参数的自由函数**,`impl App` 保留同名薄包装调用它们(同 Files 试点
   `spawn_project_git_refresh` 的"减少调用点 churn"处理):
   - `advance_rail_slot_anims(&mut self)` → `rail::advance_slot_anims(&RailLayout, &mut HashMap<PanelKind, RailSlotAnim>)`
   - `any_rail_slot_anim_active(&self)` → `rail::any_slot_anim_active(&RailLayout, &HashMap<PanelKind, RailSlotAnim>)`
   - `rail_slot_position(&self, side, kind, target_idx)` → `rail::slot_position(&HashMap<PanelKind, RailSlotAnim>, side, kind, target_idx)`
5. 已经是"薄包装调自由函数"的方法保持包装形态,只把包装的对象从
   `app.rs` 内联函数换成 `rail::` 导出函数:`rail_drag_move`(调
   `rail::rail_drag_move_into`)、`dragging_rail`(读字段,不变)、
   `dragged_panel_kind`(调 `rail::dragged_panel_kind`)、
   `rail_drag_confirmed`(调 `rail::rail_drag_past_threshold`)。
6. 四个 view 函数原样搬入(签名不变,函数体不变):`rail_icon_button`、
   `icon_rail`、`rail_drag_surface`、`rail_drag_ghost`。`app.rs` 里
   4 处调用点(`icon_rail(self, Side::Left/Right)` 两处、
   `rail_drag_ghost(self)` 一处)改成 `rail::icon_rail(self, ..)` /
   `rail::rail_drag_ghost(self)`。
7. 现有 6 个 rail 相关测试(`zone_at_x_icon_rail_returns_none`/
   `default_side_matches_rail_layout_default`/
   `panel_mirrored_false_when_rail_layout_is_default`/
   `rail_layout_default_covers_all_panels_without_duplicates`/
   `rail_layout_side_accessors_map_correctly`/
   `sanitize_rail_layout_falls_back_to_default_on_bad_data`)连同
   `dragged_panel_kind_*` 三个测试一起搬进 `rail.rs` 的
   `#[cfg(test)] mod tests`,断言不变。

**非目标**:

- 不改变任何用户可见行为、任何拖拽/动画时序或视觉效果。
- 不把 `rail_layout: RailLayout`/`rail_drag: Option<RailDrag>`/
  `rail_slot_anims: HashMap<PanelKind, RailSlotAnim>` 三个字段从 `App`/
  `ShellLayout` 移进任何私有 State——它们是内核持有的多消费方共享数据
  (呼应 Project 试点原则),继续留在 `App`/`ShellLayout` 上,只是字段
  类型改成引用 `rail::` 里的类型。
- 不动 `panel_select`、`end_rail_drag` 里跨面板编排的部分(切换
  `left_view`/`right_view`/collapsed、各面板"切入时动作"、
  `on_shell_layout_changed`)——`end_rail_drag` 里纯 Rail 部分(调用
  `rail_cross_apply`)已经是自由函数调用,搬家后改调 `rail::rail_cross_apply`
  即可,方法本身连同跨面板编排逻辑整体留在 `app.rs`。
- 不建 `Extension` trait/运行时注册表,不拆独立 crate(呼应 2026-08-20
  讨论定下的"阶段 2 倾向不做"结论)。
- 不处理 Topbar、Terminal、Preview——它们是这轮讨论定的后续候选,分别有
  各自的耦合形状(Topbar 是核心 App 状态非可选面板;Terminal/Preview 牵扯
  异步 IPC 与 wry webview),留到各自单独一轮。

## 关键语义确认(brainstorming 会话定案)

- **Rail 没有私有 `WorkspaceState`/`AppState`,不进 `extensions/`
  目录**——这是跟前 8 个试点最大的形态差异,写实现计划时不要套用
  "新建 Message 枚举 + WorkspaceState struct"的模板,`rail.rs` 就是
  类型定义 + 自由函数 + view 函数的平铺模块。
- **`rail_layout`/`rail_drag`/`rail_slot_anims` 三个字段留在 `App`/
  `ShellLayout` 上,不下放**——`rail_layout` 有 15+ 处跨 `app.rs` 的
  只读消费方(geometry/mirrored 判断/各面板 view),`rail_drag`/
  `rail_slot_anims` 虽然消费方更少但同样被 `panel_select`/
  `end_rail_drag`(跨面板编排)直接读写,不满足"私有、单一 owner"的
  extension 前提。
- **`panel_select`/`end_rail_drag` 整体留在 `app.rs`**——不是因为
  懒得拆,是因为它们的主体逻辑(切换 active view、触发各面板 reload、
  清 maximized)本来就是内核编排职责,只是恰好也会 touch 一两行 Rail
  状态。误判成"这是 Rail 的方法,应该搬"会把内核编排逻辑错误地下放,
  违反 Project 试点定下的原则。
- **`Message` 枚举不变**——`Message::RailDragMove{side,index}`/
  `Message::RailDragEnd` 两个变体、`update()` 里对应的两条分支(调用
  `self.rail_drag_move(..)`/`self.end_rail_drag()`)原样保留在
  `app.rs`,因为对应的 handler 方法本身留在 `app.rs`(见上一条)。
  这次搬迁不产生任何新的 `Message` 包装变体,纯粹是函数/类型物理位置
  移动 + 调用点加 `rail::` 前缀。

## 架构与数据流

### 1. `rail.rs` 模块内容清单

```rust
// 类型
pub enum RailButton { Panel(PanelKind), HomeProjectList, HomeRecents, HomeBrowser }
pub struct RailLayout { pub left: Vec<PanelKind>, pub right: Vec<PanelKind> }
impl RailLayout { pub fn side(..), pub fn side_mut(..), pub fn side_of(..) }
pub struct RailDrag { pub source_side, pub source_index, pub origin_index,
                       pub pending_cross_side, pub press_pos }
struct RailSlotAnim { current: f32, side: Side }
impl RailSlotAnim { fn retarget(..), fn active(..) }

// 纯函数(签名与现有 app.rs 内定义完全一致,原样搬迁)
pub(crate) fn sanitize_rail_layout(rail: RailLayout) -> RailLayout
pub(crate) fn rail_drag_move_into(rail: &mut RailLayout, drag: &mut RailDrag, side: Side, to: usize)
pub(crate) fn rail_cross_apply(rail: &mut RailLayout, source_side: Side, source_index: usize,
                                target_side: Side, target_index: usize) -> Option<PanelKind>
pub(crate) fn dragged_panel_kind(rail: &RailLayout, drag: Option<RailDrag>) -> Option<PanelKind>
pub(crate) fn rail_drag_past_threshold(drag: RailDrag, cursor: (f32, f32)) -> bool
pub(crate) fn panel_mirrored_in(rail: &RailLayout, kind: PanelKind) -> bool

// 新增自由函数(从 impl App 方法体搬出,参数化掉 self)
pub(crate) fn advance_slot_anims(rail: &RailLayout, anims: &mut HashMap<PanelKind, RailSlotAnim>)
pub(crate) fn any_slot_anim_active(rail: &RailLayout, anims: &HashMap<PanelKind, RailSlotAnim>) -> bool
pub(crate) fn slot_position(anims: &HashMap<PanelKind, RailSlotAnim>, side: Side, kind: PanelKind, target_idx: usize) -> f32

// view(签名不变,原样搬迁)
pub(crate) fn rail_icon_button<'a>(..) -> Element<'a, Message, ..>
pub(crate) fn icon_rail(app: &App, side: Side) -> Element<'_, Message, ..>
fn rail_drag_surface(..) -> Element<'_, Message, ..>
pub(crate) fn rail_drag_ghost(app: &App) -> Element<'_, Message, ..>
```

`RailSlotAnim`/`rail_drag_surface` 保持现有可见性(`struct RailSlotAnim`
私有、`fn rail_drag_surface` 模块内私有,只被 `icon_rail` 调用),其余按
现有调用点跨模块的实际需要标 `pub(crate)`。

### 2. `app.rs` 侧改动

`App` 结构体字段类型改指向 `rail::`:

```rust
struct App {
    // ...
    rail_drag: Option<rail::RailDrag>,
    rail_slot_anims: HashMap<PanelKind, rail::RailSlotAnim>,
    // ShellLayout.rail_layout: rail::RailLayout(ShellLayout 定义不变,仅字段类型变)
}
```

`impl App` 保留的薄包装(逻辑不变,只改调用目标):

```rust
fn advance_rail_slot_anims(&mut self) {
    rail::advance_slot_anims(&self.shell_layout.rail_layout, &mut self.rail_slot_anims);
}
fn any_rail_slot_anim_active(&self) -> bool {
    rail::any_slot_anim_active(&self.shell_layout.rail_layout, &self.rail_slot_anims)
}
fn rail_slot_position(&self, side: Side, kind: PanelKind, target_idx: usize) -> f32 {
    rail::slot_position(&self.rail_slot_anims, side, kind, target_idx)
}
fn rail_drag_move(&mut self, side: Side, to: usize) {
    let Some(mut drag) = self.rail_drag else { return };
    rail::rail_drag_move_into(&mut self.shell_layout.rail_layout, &mut drag, side, to);
    self.rail_drag = Some(drag);
}
pub fn dragging_rail(&self) -> bool {
    self.rail_drag.is_some()
}
pub fn dragged_panel_kind(&self) -> Option<PanelKind> {
    rail::dragged_panel_kind(&self.shell_layout.rail_layout, self.rail_drag)
}
pub fn rail_drag_confirmed(&self) -> bool {
    self.rail_drag.is_some_and(|d| rail::rail_drag_past_threshold(d, self.last_cursor))
}
```

`end_rail_drag`/`panel_select` 方法体基本原样保留在 `app.rs`,内部对
`rail_cross_apply` 的调用改成 `rail::rail_cross_apply`。

`panel_mirrored` 方法:

```rust
pub(crate) fn panel_mirrored(&self, kind: PanelKind) -> bool {
    rail::panel_mirrored_in(&self.shell_layout.rail_layout, kind)
}
```

`view()` 里三处调用点加模块前缀:`rail::icon_rail(self, Side::Left)` /
`rail::icon_rail(self, Side::Right)` / `rail::rail_drag_ghost(self)`。

`layout.rs`/`panel_layouts.rs` 里 `use crate::app::{RailLayout, ..}` 一类
的 import 改成 `use crate::rail::RailLayout`(`RailLayout` 的
`Serialize`/`Deserialize` derive 随类型一起搬,持久化 JSON 格式不变,
序列化字段名不受模块路径影响)。

### 3. 依赖方向

`rail.rs` 依赖 `app::{App, Message, PanelKind, Side}`(读 `&App` 做 view、
用 `Message` 构造拖拽消息、`PanelKind`/`Side` 是被操作的值类型),不反向
依赖任何 `extensions::*` 模块——跟现有 `region.rs`(纯样式函数库)同层级,
不是"内核依赖 extension"关系的例外。

## 错误处理

不涉及。这是纯同步 UI 状态机的代码搬迁,不新增任何可能失败的路径
(不读写文件、不发起网络/IPC 请求)。

## 测试策略

- 现有 9 个测试(6 个 rail_layout/mirrored 相关 + 3 个
  `dragged_panel_kind_*`)原样搬进 `rail.rs`,断言与测试数据不变。
- 不新增测试——这轮是代码组织重构,行为契约由"编译期防回归 + 现有测试
  全绿"验证,不是新增行为需要新覆盖。
- 人工验收:图标栏点击切换面板、按住拖拽同栏重排、跨栏拖拽换边(含
  wry webview 面板 Files/Project/Web 三个几何镜像分支)、松手确认阈值
  (`RAIL_DRAG_VISUAL_THRESHOLD_PX`)、槽位滑动动画、重启后 `rail_layout`
  持久化读回——逐项跟搬迁前手动比对一次,确认零回归。
- 编译期防回归:`cargo build -p dozer-app && cargo test -p dozer-app &&
  cargo clippy -p dozer-app --all-targets && cargo fmt --check` 全绿。

## 依赖变更

无新增依赖。
