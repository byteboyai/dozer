# Todo 左栏分类树拖动移动 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在左栏分类树里支持**拖动移动分类**（放到某分类上 = 成为其子分类；放到「全部」= 移到顶层），并删除「移动到…」选择器及 `category_picker` 整套 iced 浮层。

**Architecture:** 沿用文件树 `TreeDrag` 的两阶段机制：按下只**武装** `Pending`（不产生任何视觉反应），光标越过距离阈值且按住足够久才**确认**为 `Dragging`，`Dragging` 期间行才挂 `on_move` 上报悬停目标并画落点高亮；左键松开由窗口事件统一收尾：已确认且目标合法 → `Client::reparent_category`；未确认 → 当作一次点击（选中该分类）。拖拽合法性（自己 / 子孙 / 已是其子 / 顶层已在顶层）由纯函数判断并带单测，服务端的环检测作兜底。

**Tech Stack:** Rust 2024、iced 0.14（`MouseArea`、`mouse::Interaction::Grabbing`）、`dozer-client::reparent_category`。

**Spec:** `docs/superpowers/specs/2026-10-02-todo-webview-design.md`（「左栏分类树：拖动移动」一节）

**前置：** `docs/superpowers/plans/2026-10-02-todo-webview.md` 已执行完并合并进 main。该 plan 已经删掉了卡片分类 chip 对 `CategoryPickerOpenForTodo` / `CategoryPickerTarget::Todo` 的使用；本 plan 删除选择器剩下的全部。

## Global Constraints

- **独立 worktree 分支开发，不在 main 上直接提交**：`git worktree add .worktrees/todo-category-drag -b feat/todo-category-drag main`。主工作区常有别人未提交的改动，**不要碰、不要 stash**；每次 `git add` / `git commit` 前先 `git branch --show-current` 确认是 `feat/todo-category-drag`，只 `git add` 具体路径。提交结尾附 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- **GUI 只用 iced 0.14 生态。** 拖拽是 iced 内的交互，各平台一致，无需 `cfg` 分支。
- **不改 dozerd / dozer-client / dozer-core。** 落盘用现成的 `Client::reparent_category(category_id, new_parent_id)`（`None` = 顶层）；dozerd 的 `reparent` 把节点追加到新父节点子级末尾，目标是自己或子孙时拒绝。
- **瞬时失败走 Toast**：拖拽落盘失败经现有 `Message::CategoryMutated(Err)` → `outbox.push_err(LOG, …)` 变 Toast，不新增内联文案。日志来源 `todo`，禁止裸 `tracing` / `eprintln!`。
- **点击不能被误判成拖拽**：复用 `tree_drag_past_threshold`（12px）与 `tree_drag_held_long_enough`（300ms），两者都满足才确认；`Pending` 期间**不挂 `on_move`、不换光标、不高亮**，保证"点一下"零视觉副作用。
- **不做**兄弟节点之间的拖拽排序（继续用右键菜单「上移 / 下移」）、不做把任务卡片拖到分类上、不做幽灵跟随图标（用「被拖行半透明 + 抓取光标 + 金色落点描边」表达）。
- **测试基线**：`delete_confirm_spec_reflects_pending_target`（dozer-app）与 `cargo test --workspace` 整包下 dozerd 的 3 个 `summary_pipeline` 在 main 上本来就失败，不要修，也不要算成本 plan 的失败；宣称通过前要贴出实际命令输出。
- **`dead_code` 警告是路由 bug 的一手信号**：删除选择器后编译器报的每一条都要判断"预期要删"还是"接线漏了"。

## Review Focus

1. **点击不得变成拖拽（也不得丢失点击）**：按下→不动→松开必须仍然选中该分类；按下后微小抖动（<12px）松开同样是点击。Task 2 的 `release_action_unconfirmed_is_a_click` 与 Task 1 的阈值测试覆盖。
2. **点击展开箭头不得武装拖拽，也不得顺带选中分类**：箭头是行内的独立按钮，按下被它捕获。Task 4 的 GUI 验收覆盖（单测无法覆盖 iced 事件捕获）。
3. **成环与无意义移动**：拖到自己、自己的子孙、当前父节点、「未分类」，以及把顶层节点放到「全部」，都必须是**空操作**，不发请求。Task 1 的 `is_valid_drop_*` 覆盖。
4. **松手时目标已失效**：拖拽期间别的会话（或 agent 经 MCP）删掉了目标分类，松手时 `reparent_category` 失败 → Toast，不崩溃，状态清空。Task 2 的 `release_action_with_vanished_target_is_none` 覆盖。
5. **残留状态自愈**：拖拽中失焦、切面板、左键其实已经松开而收尾事件没到，不能让 `Pending` / `Dragging` 一直残留（之后任何悬停都被误判成拖拽）。Task 3 的 `maybe_confirm_category_drag` 在"左键未按下"时清空，并有测试。

---

## 文件结构总览

```
crates/dozer-app/src/
├── extensions/todo/
│   ├── category_drag.rs      # Task 1–2（新）：DropTarget、CategoryDrag、is_valid_drop、release_action、should_confirm
│   ├── mod.rs                # Task 1：mod category_drag; pub(crate) use
│   ├── state.rs              # Task 2：WorkspaceState.category_drag、Message 新变体
│   ├── update.rs             # Task 2：CategoryDragRelease 等处理
│   └── view.rs               # Task 4：分类行改为 MouseArea 武装/悬停
├── app/{app,update,view,message}.rs   # Task 3、5
├── platform/window_events.rs          # Task 3、5
└── …
```

---

### Task 0: 建 worktree 分支

- [x] **Step 1**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
git worktree add .worktrees/todo-category-drag -b feat/todo-category-drag main
cd .worktrees/todo-category-drag && git branch --show-current && git log --oneline -1 | cat
```
Expected: 输出 `feat/todo-category-drag`；`git log` 最近提交里应已含 Todo WebView 的合并（`git log --oneline | grep -i "todo"`）。若 main 里还没有 `extensions/todo/protocol.rs`，**停下来**：前置 plan 尚未合并。

---

### Task 1: 纯逻辑——放置目标与合法性（TDD）

**Files:**
- Create: `crates/dozer-app/src/extensions/todo/category_drag.rs`
- Modify: `crates/dozer-app/src/extensions/todo/mod.rs`（`mod category_drag; pub(crate) use category_drag::*;`）

**Interfaces:**
- Consumes: 现有 `category_descendants(categories, root) -> HashSet<i64>`（`filter.rs`，不含 `root` 自己）；`CategoryInfo { id, parent_id, … }`。
- Produces（Task 2–4 使用）：
  - `pub enum DropTarget { Root, Node(i64) }`（`Debug, Clone, Copy, PartialEq, Eq`）
  - `pub fn is_valid_drop(categories: &[CategoryInfo], source: i64, target: DropTarget) -> bool`

- [x] **Step 1: 写失败的测试**

创建 `category_drag.rs`，先只写测试模块（实现在 Step 3）：

```rust
//! 左栏分类树的拖动移动:放置目标、合法性与松手收尾的纯逻辑。机制照抄文件树
//! `TreeDrag`(按下武装 `Pending` → 越过距离+时长阈值确认 `Dragging` → 松手收尾),
//! 见 `app/layout.rs::tree_drag_past_threshold` / `tree_drag_held_long_enough`。

use super::category_descendants;
use dozer_core::protocol::CategoryInfo;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cat(id: i64, parent: Option<i64>) -> CategoryInfo {
        serde_json::from_value(json!({
            "id": id, "project_id": 1, "parent_id": parent, "name": format!("c{id}"),
            "rank": id, "created_ms": 0
        }))
        .expect("CategoryInfo")
    }

    /// 1 ─┬─ 2 ── 4
    ///    └─ 3
    /// 5(顶层)
    fn tree() -> Vec<CategoryInfo> {
        vec![
            cat(1, None),
            cat(2, Some(1)),
            cat(3, Some(1)),
            cat(4, Some(2)),
            cat(5, None),
        ]
    }

    // Review Focus 3
    #[test]
    fn cannot_drop_onto_itself() {
        assert!(!is_valid_drop(&tree(), 2, DropTarget::Node(2)));
    }

    #[test]
    fn cannot_drop_onto_own_descendant() {
        assert!(!is_valid_drop(&tree(), 1, DropTarget::Node(4)));
        assert!(!is_valid_drop(&tree(), 1, DropTarget::Node(2)));
        assert!(!is_valid_drop(&tree(), 2, DropTarget::Node(4)));
    }

    #[test]
    fn dropping_onto_current_parent_is_a_noop_and_invalid() {
        assert!(!is_valid_drop(&tree(), 2, DropTarget::Node(1)));
        assert!(!is_valid_drop(&tree(), 4, DropTarget::Node(2)));
    }

    #[test]
    fn can_drop_onto_unrelated_nodes_and_siblings() {
        assert!(is_valid_drop(&tree(), 2, DropTarget::Node(3))); // 兄弟:变成它的子分类
        assert!(is_valid_drop(&tree(), 4, DropTarget::Node(5))); // 换到另一棵树
        assert!(is_valid_drop(&tree(), 5, DropTarget::Node(1)));
        assert!(is_valid_drop(&tree(), 4, DropTarget::Node(1))); // 上移一层
    }

    #[test]
    fn root_target_is_valid_only_when_the_node_is_not_already_top_level() {
        assert!(is_valid_drop(&tree(), 2, DropTarget::Root));
        assert!(is_valid_drop(&tree(), 4, DropTarget::Root));
        assert!(!is_valid_drop(&tree(), 1, DropTarget::Root));
        assert!(!is_valid_drop(&tree(), 5, DropTarget::Root));
    }

    #[test]
    fn unknown_source_or_target_is_invalid() {
        assert!(!is_valid_drop(&tree(), 99, DropTarget::Root));
        assert!(!is_valid_drop(&tree(), 99, DropTarget::Node(1)));
        assert!(!is_valid_drop(&tree(), 2, DropTarget::Node(99)));
    }
}
```

- [x] **Step 2: 跑测试确认失败**

先在 `mod.rs` 加 `mod category_drag;` 与 `pub(crate) use category_drag::*;`，然后：

Run: `cargo test -p dozer-app extensions::todo::category_drag 2>&1 | tail -8`
Expected: 编译失败，`cannot find type DropTarget` / `function is_valid_drop`（RED）。

- [x] **Step 3: 实现**

在 `category_drag.rs` 里、`mod tests` 之前加：

```rust
/// 拖拽放置目标:顶层(「全部」伪节点)或某个真实分类节点。「未分类」伪节点
/// 不是合法目标(它不对应真实节点),所以没有对应变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropTarget {
    Root,
    Node(i64),
}

/// `source` 能否放到 `target`。无意义的移动(放到当前父节点、顶层节点放到顶层)
/// 一律判不合法——UI 不高亮、松手空操作,也就不发请求。服务端 `reparent`
/// 仍会再校验一次环检测,双保险。
pub fn is_valid_drop(categories: &[CategoryInfo], source: i64, target: DropTarget) -> bool {
    let Some(src) = categories.iter().find(|c| c.id == source) else {
        return false;
    };
    match target {
        DropTarget::Root => src.parent_id.is_some(),
        DropTarget::Node(t) => {
            if t == source || !categories.iter().any(|c| c.id == t) {
                return false;
            }
            if src.parent_id == Some(t) {
                return false;
            }
            !category_descendants(categories, source).contains(&t)
        }
    }
}
```

- [x] **Step 4: 跑测试确认通过并提交**

Run: `cargo test -p dozer-app extensions::todo::category_drag 2>&1 | tail -8`
Expected: 6 个 PASS。

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/todo/category_drag.rs crates/dozer-app/src/extensions/todo/mod.rs
git commit -m "feat(todo): category drag-and-drop validity (pure logic)" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: 拖拽状态机与松手收尾（TDD）

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo/category_drag.rs`（`CategoryDrag`、`CategoryDragPhase`、`should_confirm`、`ReleaseAction`、`release_action`）
- Modify: `crates/dozer-app/src/extensions/todo/state.rs`（字段、方法、`Message` 变体）
- Modify: `crates/dozer-app/src/extensions/todo/update.rs`（处理 `CategoryDragRelease` 等）

**Interfaces:**
- Consumes: Task 1 的 `DropTarget`、`is_valid_drop`；`app/layout.rs` 的 `tree_drag_past_threshold`、`tree_drag_held_long_enough`（经 `crate::app::` 可达，路径以编译器为准）。
- Produces:
  - `pub enum CategoryDragPhase { Pending, Dragging }`
  - `pub struct CategoryDrag { pub source: i64, pub phase: CategoryDragPhase, pub press_pos: (f32, f32), pub armed_at: std::time::Instant, pub over: Option<DropTarget> }`
  - `pub fn should_confirm(drag: &CategoryDrag, cursor: (f32, f32), now: Instant) -> bool`
  - `pub enum ReleaseAction { Select(i64), Reparent { id: i64, new_parent: Option<i64> }, Nothing }`
  - `pub fn release_action(drag: &CategoryDrag, categories: &[CategoryInfo]) -> ReleaseAction`
  - `WorkspaceState` 方法：`arm_category_drag(source, press_pos, armed_at)`、`category_drag() -> Option<&CategoryDrag>`、`category_drag_confirmed() -> bool`、`confirm_category_drag()`、`cancel_category_drag()`、`set_category_drag_over(Option<DropTarget>)`、`take_category_drag() -> Option<CategoryDrag>`
  - `Message::{CategoryRowPress(i64), CategoryDragOver(Option<DropTarget>), CategoryDragRelease}`

- [x] **Step 1: 写失败的测试**

在 `category_drag.rs` 的 `mod tests` 追加：

```rust
    use std::time::{Duration, Instant};

    fn drag(source: i64, phase: CategoryDragPhase, over: Option<DropTarget>) -> CategoryDrag {
        CategoryDrag {
            source,
            phase,
            press_pos: (100.0, 100.0),
            armed_at: Instant::now(),
            over,
        }
    }

    // Review Focus 1
    #[test]
    fn should_confirm_needs_distance_and_hold_time() {
        let d = CategoryDrag {
            armed_at: Instant::now() - Duration::from_millis(500),
            ..drag(2, CategoryDragPhase::Pending, None)
        };
        assert!(should_confirm(&d, (100.0, 130.0), Instant::now()));
        // 距离不够(抖动)
        assert!(!should_confirm(&d, (103.0, 102.0), Instant::now()));
        // 按住时间不够
        let fresh = drag(2, CategoryDragPhase::Pending, None);
        assert!(!should_confirm(&fresh, (100.0, 130.0), fresh.armed_at + Duration::from_millis(50)));
    }

    #[test]
    fn already_dragging_is_not_reconfirmed() {
        let d = CategoryDrag {
            armed_at: Instant::now() - Duration::from_millis(500),
            ..drag(2, CategoryDragPhase::Dragging, None)
        };
        assert!(!should_confirm(&d, (100.0, 300.0), Instant::now()));
    }

    // Review Focus 1:没确认就松手 = 点击,选中该分类。
    #[test]
    fn release_action_unconfirmed_is_a_click() {
        let d = drag(2, CategoryDragPhase::Pending, None);
        assert_eq!(release_action(&d, &tree()), ReleaseAction::Select(2));
        // 即使 over 里有值,Pending 也只是点击
        let d = drag(2, CategoryDragPhase::Pending, Some(DropTarget::Node(5)));
        assert_eq!(release_action(&d, &tree()), ReleaseAction::Select(2));
    }

    #[test]
    fn release_action_confirmed_with_valid_target_reparents() {
        let d = drag(4, CategoryDragPhase::Dragging, Some(DropTarget::Node(5)));
        assert_eq!(
            release_action(&d, &tree()),
            ReleaseAction::Reparent { id: 4, new_parent: Some(5) }
        );
        let d = drag(4, CategoryDragPhase::Dragging, Some(DropTarget::Root));
        assert_eq!(
            release_action(&d, &tree()),
            ReleaseAction::Reparent { id: 4, new_parent: None }
        );
    }

    #[test]
    fn release_action_confirmed_without_or_with_invalid_target_does_nothing() {
        let none = drag(2, CategoryDragPhase::Dragging, None);
        assert_eq!(release_action(&none, &tree()), ReleaseAction::Nothing);
        let onto_self = drag(2, CategoryDragPhase::Dragging, Some(DropTarget::Node(2)));
        assert_eq!(release_action(&onto_self, &tree()), ReleaseAction::Nothing);
        let onto_child = drag(1, CategoryDragPhase::Dragging, Some(DropTarget::Node(4)));
        assert_eq!(release_action(&onto_child, &tree()), ReleaseAction::Nothing);
    }

    // Review Focus 4:松手时目标已被删掉。
    #[test]
    fn release_action_with_vanished_target_is_none() {
        let d = drag(4, CategoryDragPhase::Dragging, Some(DropTarget::Node(77)));
        assert_eq!(release_action(&d, &tree()), ReleaseAction::Nothing);
    }

    #[test]
    fn release_action_with_vanished_source_is_nothing() {
        let d = drag(99, CategoryDragPhase::Dragging, Some(DropTarget::Root));
        assert_eq!(release_action(&d, &tree()), ReleaseAction::Nothing);
        // 点击一个已不存在的行不选中任何东西
        let click = drag(99, CategoryDragPhase::Pending, None);
        assert_eq!(release_action(&click, &tree()), ReleaseAction::Nothing);
    }
```

在 `extensions/todo/mod.rs` 的 `mod tests` 追加状态方法的测试：

```rust
    #[test]
    fn category_drag_lifecycle_arm_confirm_over_take() {
        let mut ws_state = WorkspaceState::default();
        assert!(ws_state.category_drag().is_none());
        ws_state.arm_category_drag(7, (10.0, 10.0), std::time::Instant::now());
        assert!(!ws_state.category_drag_confirmed());
        // Pending 期间设置悬停目标无效(不允许有任何反应)
        ws_state.set_category_drag_over(Some(DropTarget::Root));
        assert_eq!(ws_state.category_drag().unwrap().over, None);
        ws_state.confirm_category_drag();
        assert!(ws_state.category_drag_confirmed());
        ws_state.set_category_drag_over(Some(DropTarget::Node(3)));
        assert_eq!(ws_state.category_drag().unwrap().over, Some(DropTarget::Node(3)));
        let taken = ws_state.take_category_drag().unwrap();
        assert_eq!(taken.source, 7);
        assert!(ws_state.category_drag().is_none());
    }

    #[test]
    fn cancel_category_drag_clears_any_phase() {
        let mut ws_state = WorkspaceState::default();
        ws_state.arm_category_drag(7, (0.0, 0.0), std::time::Instant::now());
        ws_state.cancel_category_drag();
        assert!(ws_state.category_drag().is_none());
        ws_state.cancel_category_drag(); // 幂等
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app extensions::todo 2>&1 | grep -E "^error" | head -5`
Expected: 一批 `cannot find ...`（RED）。

- [x] **Step 3: 实现纯函数与类型**

`category_drag.rs`（`mod tests` 之前）：

```rust
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryDragPhase {
    /// 已按下武装,但还没越过阈值:此时**不得有任何视觉反应**("点一下"零副作用)。
    Pending,
    /// 已确认是一次真实拖拽:行才挂 `on_move`、换抓取光标、画落点高亮。
    Dragging,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CategoryDrag {
    pub source: i64,
    pub phase: CategoryDragPhase,
    /// 按下瞬间的光标位置(逻辑像素),用来过距离阈值。
    pub press_pos: (f32, f32),
    pub armed_at: Instant,
    /// 当前悬停的**合法**放置目标(非法目标不记录)。
    pub over: Option<DropTarget>,
}

/// `Pending → Dragging` 的确认条件:距离与按住时长两道阈值都越过(同文件树)。
pub fn should_confirm(drag: &CategoryDrag, cursor: (f32, f32), now: Instant) -> bool {
    drag.phase == CategoryDragPhase::Pending
        && crate::app::tree_drag_past_threshold(drag.press_pos, cursor)
        && crate::app::tree_drag_held_long_enough(now.saturating_duration_since(drag.armed_at))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseAction {
    /// 没确认就松手 = 点击:选中该分类。
    Select(i64),
    Reparent { id: i64, new_parent: Option<i64> },
    Nothing,
}

/// 松手收尾:未确认 → 点击(源行仍存在才选中);已确认 → 目标合法才移动,否则空操作。
/// 源或目标在拖拽期间被删掉(别的会话 / agent)一律落到 `Nothing`。
pub fn release_action(drag: &CategoryDrag, categories: &[CategoryInfo]) -> ReleaseAction {
    if !categories.iter().any(|c| c.id == drag.source) {
        return ReleaseAction::Nothing;
    }
    if drag.phase == CategoryDragPhase::Pending {
        return ReleaseAction::Select(drag.source);
    }
    match drag.over {
        Some(target) if is_valid_drop(categories, drag.source, target) => ReleaseAction::Reparent {
            id: drag.source,
            new_parent: match target {
                DropTarget::Root => None,
                DropTarget::Node(id) => Some(id),
            },
        },
        _ => ReleaseAction::Nothing,
    }
}
```

- [x] **Step 4: 实现状态方法与消息**

`state.rs`：`WorkspaceState` 增字段（放在 `category_renaming` 附近）：

```rust
    /// 分类树拖动移动的进行态(`None` = 没在拖)。见 `category_drag::CategoryDrag`。
    pub(crate) category_drag: Option<CategoryDrag>,
```

`impl WorkspaceState` 增：

```rust
    /// 分类行被按下:武装拖拽(`Pending`)。该消息由内核拦截(要拿 `last_cursor`),
    /// 内核直接调用这个方法。
    pub(crate) fn arm_category_drag(
        &mut self,
        source: i64,
        press_pos: (f32, f32),
        armed_at: std::time::Instant,
    ) {
        self.category_drag = Some(CategoryDrag {
            source,
            phase: CategoryDragPhase::Pending,
            press_pos,
            armed_at,
            over: None,
        });
    }

    pub(crate) fn category_drag(&self) -> Option<&CategoryDrag> {
        self.category_drag.as_ref()
    }

    /// 已确认(`Dragging`)——视图层据此给行挂 `on_move`、换抓取光标、画高亮。
    /// `Pending` 期间返回 `false`:此时必须完全没有反应。
    pub(crate) fn category_drag_confirmed(&self) -> bool {
        self.category_drag
            .as_ref()
            .is_some_and(|d| d.phase == CategoryDragPhase::Dragging)
    }

    pub(crate) fn confirm_category_drag(&mut self) {
        if let Some(d) = self.category_drag.as_mut() {
            d.phase = CategoryDragPhase::Dragging;
        }
    }

    pub(crate) fn cancel_category_drag(&mut self) {
        self.category_drag = None;
    }

    /// 光标悬停到某个候选目标。`Pending` 期间忽略;`target` 不合法时清空记录
    /// (UI 不高亮非法目标),所以 `over` 里永远只有合法目标。
    pub(crate) fn set_category_drag_over(&mut self, target: Option<DropTarget>) {
        let Some(d) = self.category_drag.as_mut() else {
            return;
        };
        if d.phase != CategoryDragPhase::Dragging {
            return;
        }
        d.over = target.filter(|t| is_valid_drop(&self.categories, d.source, *t));
    }

    pub(crate) fn take_category_drag(&mut self) -> Option<CategoryDrag> {
        self.category_drag.take()
    }
```

> `set_category_drag_over` 里同时借 `self.category_drag`（可变）与 `self.categories`（不可变）会触发借用冲突；先 `let source = d.source;` 释放可变借用再算 `is_valid_drop`，或把 `categories` 先 clone 引用——按编译器提示调整，行为不变。

`Message` 增：

```rust
    /// 分类行被按下(武装拖拽;内核拦截以取 `last_cursor`)。
    CategoryRowPress(i64),
    /// 拖拽已确认期间光标悬停到候选目标(`None` = 离开所有目标)。
    CategoryDragOver(Option<DropTarget>),
    /// 左键松开的收尾(窗口事件在 `dragging_category()` 时发)。
    CategoryDragRelease,
```

`update.rs`：

```rust
        Message::CategoryRowPress(_) => {} // 由内核拦截武装,不进 `update`
        Message::CategoryDragOver(target) => ws_state.set_category_drag_over(target),
        Message::CategoryDragRelease => {
            let Some(drag) = ws_state.take_category_drag() else {
                return;
            };
            match release_action(&drag, &ws_state.categories) {
                ReleaseAction::Select(id) => {
                    ws_state.category_selected = CategoryFilter::Node(id);
                }
                ReleaseAction::Reparent { id, new_parent } => {
                    let client = client.clone();
                    handle.spawn(async move {
                        let res = client
                            .reparent_category(id, new_parent)
                            .await
                            .map(|_| ())
                            .map_err(|e| e.to_string());
                        emit(Message::CategoryMutated(res));
                    });
                }
                ReleaseAction::Nothing => {}
            }
        }
```

> 点击选中要与原 `CategorySelect(filter)` 的副作用一致（它还会 `ws_state.category_selected = filter; ws_state.clear_search()`）。**Plan 1 之后 `clear_search` 已被删除**（搜索清空由前端按 `category_key` 变化完成），所以这里只设 `category_selected` 即可；若合并后的 main 里 `CategorySelect` 还有别的副作用，让 `Select` 分支直接复用同一段（把 `CategorySelect` 的处理抽成 `fn select_category(ws_state, filter)` 两处共用，不要复制）。

- [x] **Step 5: 跑测试**

Run: `cargo test -p dozer-app extensions::todo 2>&1 | tail -12`
Expected: 全部 PASS（含新增约 12 个）。

- [x] **Step 6: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/todo
git commit -m "feat(todo): category drag state machine, release action and messages" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: 内核接线——确认、松手收尾、Esc 取消、残留自愈

**Files:**
- Modify: `crates/dozer-app/src/app/app.rs`（`maybe_confirm_category_drag`、`dragging_category`、`category_drag_confirmed`）
- Modify: `crates/dozer-app/src/app/update.rs`（拦截 `CategoryRowPress` 武装）
- Modify: `crates/dozer-app/src/platform/window_events.rs`（`CursorMoved` 钩子、左键松开路由、Esc 取消）

**Interfaces:**
- Consumes: Task 2 的 `WorkspaceState` 方法与 `should_confirm`。
- Produces: `App::dragging_category() -> bool`、`App::category_drag_confirmed() -> bool`、`App::maybe_confirm_category_drag(left_mouse_down: bool) -> bool`。

- [x] **Step 1: 写失败的测试（残留自愈）**

`App` 难以在单测里构造，把"左键没按下时清空、按下且越阈值才确认"的决策抽成可测的纯函数。在 `category_drag.rs` 加测试：

```rust
    // Review Focus 5:左键并未按下却残留了拖拽态 → 必须清空,不能确认。
    #[test]
    fn tick_clears_a_stale_drag_when_the_mouse_is_not_down() {
        let d = CategoryDrag {
            armed_at: Instant::now() - Duration::from_millis(500),
            ..drag(2, CategoryDragPhase::Pending, None)
        };
        assert_eq!(tick(Some(&d), false, (100.0, 300.0), Instant::now()), Tick::Clear);
    }

    #[test]
    fn tick_confirms_only_with_the_mouse_down_and_both_thresholds() {
        let d = CategoryDrag {
            armed_at: Instant::now() - Duration::from_millis(500),
            ..drag(2, CategoryDragPhase::Pending, None)
        };
        assert_eq!(tick(Some(&d), true, (100.0, 300.0), Instant::now()), Tick::Confirm);
        assert_eq!(tick(Some(&d), true, (101.0, 101.0), Instant::now()), Tick::Nothing);
        assert_eq!(tick(None, true, (0.0, 0.0), Instant::now()), Tick::Nothing);
        assert_eq!(tick(None, false, (0.0, 0.0), Instant::now()), Tick::Nothing);
    }
```

Run: `cargo test -p dozer-app extensions::todo::category_drag 2>&1 | grep -E "^error" | head -3`
Expected: `cannot find function tick` / `Tick`（RED）。

- [x] **Step 2: 实现 `tick`**

`category_drag.rs`：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    Nothing,
    /// 左键没按下却还有残留拖拽态:自愈清空。
    Clear,
    /// 该推进到 `Dragging`。
    Confirm,
}

/// 每次 `CursorMoved` 的决策(纯函数,便于测试):没有物理按住左键就绝不确认,
/// 并清掉任何残留(正常路径下松手收尾早该清过;还留着只可能是那次收尾丢了)。
pub fn tick(
    drag: Option<&CategoryDrag>,
    left_mouse_down: bool,
    cursor: (f32, f32),
    now: Instant,
) -> Tick {
    let Some(d) = drag else {
        return Tick::Nothing;
    };
    if !left_mouse_down {
        return Tick::Clear;
    }
    if should_confirm(d, cursor, now) {
        Tick::Confirm
    } else {
        Tick::Nothing
    }
}
```

Run: `cargo test -p dozer-app extensions::todo::category_drag 2>&1 | tail -6`
Expected: PASS。

- [x] **Step 3: `App` 方法**

`app/app.rs`（紧挨 `maybe_confirm_tree_drag`）：

```rust
    /// 分类树拖动移动是否有进行中的拖拽(含 `Pending`)——窗口事件据此决定要不要在
    /// 左键松开时发收尾消息(同 `dragging_tree_item`)。
    pub(crate) fn dragging_category(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.category_drag().is_some())
    }

    /// 已确认(`Dragging`)——供顶层判断要不要重绘 / 换光标。
    pub(crate) fn category_drag_confirmed(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.category_drag_confirmed())
    }

    /// 每次 `CursorMoved` 调用。返回是否发生了状态变化(调用方据此请求重绘)。
    pub(crate) fn maybe_confirm_category_drag(&mut self, left_mouse_down: bool) -> bool {
        let cursor = self.last_cursor;
        let action = {
            let Some(ws) = self.active_workspace() else {
                return false;
            };
            todo::tick(
                ws.todo.category_drag(),
                left_mouse_down,
                cursor,
                std::time::Instant::now(),
            )
        };
        let Some(ws) = self.active_workspace_mut() else {
            return false;
        };
        match action {
            todo::Tick::Nothing => false,
            todo::Tick::Clear => {
                ws.todo.cancel_category_drag();
                true
            }
            todo::Tick::Confirm => {
                ws.todo.confirm_category_drag();
                true
            }
        }
    }
```

`app/update.rs` 里 `Message::Todo(msg) => match msg { … }` 增一条分支（紧挨 `CategoryContextMenuOpen`），内核拦截以取 `last_cursor`：

```rust
                todo::Message::CategoryRowPress(id) => {
                    let press_pos = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.todo
                            .arm_category_drag(id, press_pos, std::time::Instant::now());
                    });
                }
```

- [x] **Step 4: 窗口事件接线**

`platform/window_events.rs`：

1. `CursorMoved` 分支里、`if app.maybe_confirm_tree_drag(*left_mouse_down) { window.request_redraw(); }` 之后加：

```rust
                if app.maybe_confirm_category_drag(*left_mouse_down) {
                    window.request_redraw();
                }
```

2. 在"文件树内拖拽移动同理"那个 `MouseInput { Released, Left } if app.dragging_tree_item()` 分支之后加：

```rust
            // 分类树拖动移动同理:左键松开即收尾——已确认且目标合法就移动,未确认
            // 则当作一次点击选中该分类(见 `category_drag::release_action`)。
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: winit::event::MouseButton::Left,
                ..
            } if app.dragging_category() => {
                app.update(Message::Todo(
                    crate::extensions::todo::Message::CategoryDragRelease,
                ));
                window.request_redraw();
            }
```

3. 在 Esc 关弹层的那串 `if … && event.logical_key == Key::Named(NamedKey::Escape)` 块里加一个（放在其它 Esc 块旁边，口径相同）：

```rust
        // 分类树拖动进行中按 Esc:取消拖拽(不移动、也不当作点击选中)。
        if app.dragging_category()
            && let WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } = event
            && event.state == ElementState::Pressed
            && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            if let Some(ws) = app.active_workspace_mut() {
                ws.todo.cancel_category_drag();
            }
            window.request_redraw();
            return false;
        }
```

- [x] **Step 4b: 切面板 / 失焦时清空**

在 `workspace/state.rs` 里原来调用 `self.todo.cancel_drag()` 的位置（Plan 1 已删除该行；找"切面板 / 失焦时清拖拽状态"的同类函数，通常叫 `cancel_pending_drags` 之类）加上 `self.todo.cancel_category_drag();`。若找不到这样的函数，不要新造，依赖 `maybe_confirm_category_drag` 的"左键未按下即清空"自愈（Review Focus 5）。

- [x] **Step 5: 编译与测试**

```bash
cargo build 2>&1 | grep -E "^error" -A8 | head -20
cargo test -p dozer-app extensions::todo 2>&1 | tail -6
```
Expected: 编译通过；测试 PASS。此时 `CategoryRowPress` 还没有发出方（Task 4），`dead_code` 提示属预期。

- [x] **Step 6: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src
git commit -m "feat(todo): wire category drag confirmation, release routing and Esc cancel" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 视图——分类行改为"按下武装 + 拖动高亮"

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo/view.rs`（`category_tree_nav`）

**Interfaces:**
- Consumes: `Message::CategoryRowPress`、`CategoryDragOver`；`ws_state.category_drag()`、`category_drag_confirmed()`；`DropTarget`。

要点（照抄文件树的做法，见 `extensions/files/view.rs` 340–470 行的长注释）：iced 的 `button::on_press` 实际在 `ButtonReleased` 才发，所以**行内的 `button` 不接 `on_press`**，改由外层 `MouseArea::on_press` 在物理按下那一刻武装；点击选中改由松手收尾（`CategoryDragRelease` → `ReleaseAction::Select`）完成。

- [x] **Step 1: 改 `category_tree_nav` 的真实节点行**

在 `for row in rows { … }` 循环里，把

```rust
        .on_press(Message::CategorySelect(CategoryFilter::Node(row.id)))
```
从 `label` 这个 `button` 上**删掉**（`button` 其余样式不变；chevron 的 `icon_button_entry` 保持原样，它是行内独立按钮，按下被它捕获，不会冒泡武装拖拽）。并在构造样式之前算好拖拽相关状态：

```rust
        let drag = ws_state.category_drag();
        let is_source = drag.is_some_and(|d| d.source == row.id)
            && ws_state.category_drag_confirmed();
        let is_drop_target = drag.is_some_and(|d| d.over == Some(DropTarget::Node(row.id)));
```

`label` 的样式闭包里：被拖行半透明（`is_source` 时整体 alpha 0.4），合法落点描边为金色（沿用文件树落点高亮的 `gold` 1px，`is_drop_target` 时覆盖 `active` 的边框）：

```rust
        .style(move |_t: &iced_widget::Theme, _s| button::Style {
            background: if active {
                Some(byteui::theme::color::current().card.into())
            } else {
                None
            },
            text_color: if is_source {
                Color { a: 0.4, ..fg }
            } else {
                fg
            },
            border: Border {
                color: if is_drop_target || active {
                    byteui::theme::color::current().gold
                } else {
                    Color::TRANSPARENT
                },
                width: if is_drop_target || active { 1.0 } else { 0.0 },
                radius: 6.0.into(),
            },
            ..button::Style::default()
        });
```

把 `label.into()` 外层先包一层拖拽用的 `MouseArea`，再交给右键菜单 `context_menu::wrap`：

```rust
        let mut area = iced_widget::MouseArea::new(label).on_press(Message::CategoryRowPress(row.id));
        // 已确认(`Dragging`)才挂 `on_move` 与抓取光标;`Pending` 期间必须完全没有
        // 反应("点一下就进入拖拽态"的根因,见文件树 `TreeDragPhase` 文档)。
        if ws_state.category_drag_confirmed() {
            let target = DropTarget::Node(row.id);
            area = area
                .on_move(move |_| Message::CategoryDragOver(Some(target)))
                .interaction(iced_widget::core::mouse::Interaction::Grabbing);
        }
        col = col.push(byteui::interaction::context_menu::wrap(
            area.into(),
            Some(Message::CategoryContextMenuOpen(Some(row.id))),
        ));
```

- [x] **Step 2: 「全部」伪节点做顶层放置目标**

「全部」行仍是 `category_pseudo_row(..., Message::CategorySelect(CategoryFilter::All))`（点击由它自己的 `button` 处理，不参与武装）。为它加落点高亮和 `on_move`：把「全部」那一处改成

```rust
    let root_is_drop_target = ws_state
        .category_drag()
        .is_some_and(|d| d.over == Some(DropTarget::Root));
    let mut all_area = iced_widget::MouseArea::new(category_pseudo_row(
        icons::IconKind::CircleSmall,
        "全部",
        ws_state.category_selected() == CategoryFilter::All,
        Message::CategorySelect(CategoryFilter::All),
    ));
    if ws_state.category_drag_confirmed() {
        all_area = all_area
            .on_move(|_| Message::CategoryDragOver(Some(DropTarget::Root)))
            .interaction(iced_widget::core::mouse::Interaction::Grabbing);
    }
    // 合法的"移到顶层"落点:整行金色描边(与节点落点同一视觉语言)。
    let all_el: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if root_is_drop_target {
            container(all_area)
                .style(|_t: &iced_widget::Theme| container::Style {
                    border: Border {
                        color: byteui::theme::color::current().gold,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                })
                .into()
        } else {
            all_area.into()
        };
    col = col.push(byteui::interaction::context_menu::wrap(
        all_el,
        Some(Message::CategoryContextMenuOpen(None)),
    ));
```

「未分类」行**不接** `on_move`、不高亮（它不是合法放置目标）。

- [x] **Step 3: 光标离开所有目标时清空悬停目标**

拖拽确认期间，光标移到树外 / 「未分类」/ 空白处，`over` 应清空，否则会停留在最后一个目标上，松手时误落。给 `col` 外层（`category_tree_nav` 返回前）包一层：已确认时挂 `on_exit(Message::CategoryDragOver(None))`：

```rust
    let tree: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = col.into();
    if ws_state.category_drag_confirmed() {
        iced_widget::MouseArea::new(tree)
            .on_exit(Message::CategoryDragOver(None))
            .into()
    } else {
        tree
    }
```
（把原来末尾的 `col.into()` 改为上面这段。）「未分类」行本身没有 `on_move`，光标在其上时 `over` 保持上一个目标——为避免该情况，也给「未分类」行包 `MouseArea::new(...).on_enter(Message::CategoryDragOver(None))`（仅在 `category_drag_confirmed()` 时挂）。

- [x] **Step 4: 编译并回归**

```bash
cargo build 2>&1 | grep -E "^error" -A8 | head -30
cargo test -p dozer-app extensions::todo 2>&1 | tail -6
```
Expected: 编译通过；测试 PASS。`CategorySelect(Node)` 变体仍被「全部 / 未分类」的 `button` 与松手点击使用，不应出现 `dead_code`。

- [x] **Step 5: GUI 验收（需要显示环境；无法执行时在 ledger 明确写"未做 GUI 验收"）**

> 未做 GUI 验收：本环境无显示环境，下列 8 项未能逐项人工执行。自动测试（`is_valid_drop` / `release_action` / `tick` / 状态机 15 项）、`cargo build`、`cargo fmt`、clippy、workspace 测试与 `check-log-scope.sh` 均已通过。

```bash
cargo run -p dozer-app
```
先在 Todo 左栏建几个分类（右键「新建分类 / 新建子分类」），例如：`后端 ▸ 接口`、`前端`，再逐项：

1. **点击仍然选中**：单击某分类 → 选中并过滤右栏；按下后手抖动几像素再松开 → 仍是点击选中。
2. **展开箭头**：点箭头只展开 / 收起，**不改变选中**，也不进入拖拽。
3. **拖到另一个分类上**：按住「接口」拖到「前端」上，期间「前端」金色描边、「接口」变淡、光标变抓取；松手后「接口」成为「前端」的子分类（树刷新）。
4. **拖到「全部」上**：把子分类拖到「全部」，「全部」整行金色描边；松手后移到顶层。顶层分类拖到「全部」上则**无高亮**、松手无变化。
5. **非法目标**：拖「后端」到它自己或它的子孙「接口」上，**无高亮**，松手无变化、无报错。
6. **拖到「未分类」/ 树外松手**：无变化。
7. **Esc 取消**：拖拽中按 Esc，松手后不移动、也不选中。
8. **失败路径**：在拖拽过程中让 agent（MCP）或另一个窗口删除目标分类，再松手 → 一条 Toast，列表刷新，不崩溃。

- [x] **Step 6: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/todo/view.rs
git commit -m "feat(todo): drag category rows to reparent (grab cursor, drop highlight, root target)" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: 删除「移动到…」与 `category_picker` 整套

**Files:**
- Modify: `crates/dozer-app/src/app/{app,update,view,message}.rs`
- Modify: `crates/dozer-app/src/platform/window_events.rs`
- Modify: `crates/dozer-app/src/extensions/todo/{state,update}.rs`

- [x] **Step 1: 删右键菜单里的「移动到…」项**

- `app/app.rs`（约 230 行，macOS 原生菜单 `category_context_menu_items`）：删除 `"移动到..."` 这一个 `Item::Entry`（连同 `Message::Todo(todo::Message::CategoryReparentPickerOpen(id))`）；若删除后紧邻的分隔线变成孤立 / 重复，一并整理。
- `app/update.rs`（约 6237 行，非 macOS 的 iced 菜单 `category_context_menu_popup`）：同样删除该项；并更新注释里"下移、移动到..."的描述为"上移、下移"。

- [x] **Step 2: 删选择器整套**

- `app/message.rs`：删除 `CategoryPickerClose`、`CategoryPickerSelect(Option<i64>)`，以及 `CategoryPickerTarget` 枚举与 `CategoryPicker` 结构体（约 776–789 行）。
- `app/app.rs`：删除字段 `category_picker`（约 400 行）及其初始化（约 827 行）、`category_picker_open()`（约 3037 行）、`todo_category_picker_open`（约 3047 行）；`App` 的 `use` 里不再需要的 `CategoryPicker` / `CategoryPickerTarget` 一并清理。
- `app/update.rs`：删除 `todo::Message::CategoryReparentPickerOpen(id) => { … }` 分支（约 1770 行）、`Message::CategoryPickerClose => …` 与 `Message::CategoryPickerSelect(chosen) => …` 两条分支（约 2863–2900 行）、`category_picker_popup` 方法（约 6273 行起整个函数及其文档注释）。
- `app/view.rs`：删除 `else if self.category_picker.is_some() { … }` 分支（约 194–204 行：`dismiss` + `stack![base, dismiss, self.category_picker_popup()]`）。
- `platform/window_events.rs`：删除 Esc 路由里 `else if app.category_picker_open() { app.update(Message::CategoryPickerClose); }`（约 691 行），保持其它 `else if` 链完整。
- `extensions/todo/state.rs` 与 `update.rs`：删除 `Message::CategoryReparentPickerOpen(i64)` 变体及 `update` 里的空分支；Plan 1 之后 `CategoryPickerOpenForTodo` 应已删除，若还在一并删。

- [x] **Step 3: 门禁与测试**

```bash
cargo build 2>&1 | grep -E "^(error|warning: unused|warning: .*never)" -A6 | head -30
grep -rnE "category_picker|CategoryPicker|CategoryReparentPickerOpen|CategoryPickerOpenForTodo|移动到\.\.\." crates --include='*.rs' | grep -v "^[^:]*:[0-9]*:\s*//"
cargo test -p dozer-app 2>&1 | grep -E "^test result|FAILED" | head
bash scripts/check-log-scope.sh
```
Expected: 编译无 `dead_code` / `unused` 警告；grep **无输出**；`dozer-app` 测试只剩基线那 1 个失败；门禁 ok。

- [x] **Step 4: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src
git commit -m "refactor(todo): remove the 'move to…' menu item and the iced category picker" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: 最终验证与收尾

- [x] **Step 1: 全量检查**

```bash
cargo fmt --check && echo fmt-ok
cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "generated [0-9]+ warning" | head -3
cargo test --workspace --no-fail-fast 2>&1 | grep -E "FAILED"
bash scripts/check-log-scope.sh
```
Expected: fmt ok；clippy 警告数不高于 main 基线；失败只有已知基线（`delete_confirm_spec_reflects_pending_target` 与 dozerd 的 3 个 `summary_pipeline`）；门禁 ok。

- [x] **Step 2: 人工验收**

重跑 Task 4 Step 5 的 8 项；另外确认：右键分类菜单（macOS 原生）里**没有**「移动到…」，仍有「新建子 / 同级、上移、下移、重命名、删除」；右键「上移 / 下移」仍然有效。

- [x] **Step 3: 更新 spec 状态并收尾**

在 `docs/superpowers/specs/2026-10-02-todo-webview-design.md` 的「左栏分类树：拖动移动」一节末尾补一句实现说明（沿用文件树 `TreeDrag` 机制、阈值 12px + 300ms、`ReleaseAction` 纯函数），提交：

```bash
git branch --show-current
git add docs/superpowers/specs/2026-10-02-todo-webview-design.md
git commit -m "docs: note the category drag implementation in the todo webview spec" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

按 `superpowers:finishing-a-development-branch` 处理，**合并到 main 由用户决定**；合并前检查主工作区别人未提交的改动是否与本分支文件重叠，有重叠按"唯一标签 stash → 快进 → `stash apply` → 按标签 drop"处理，不碰别人的 stash。

---

## 自检记录

- **Spec 覆盖**（分类拖动一节）：拖起（两阶段 + 12px/300ms）→ Task 2、3；放下 = 成为子分类 → Task 2（`Reparent{Some}`）；放到「全部」= 顶层、「未分类」不接受 → Task 1、4；防成环与无意义移动 → Task 1；视觉（金色落点、被拖行半透明、抓取光标）→ Task 4；删除「移动到…」两处菜单与 `category_picker` 整套 → Task 5；失败走 Toast → Task 2（`CategoryMutated`）。
- **类型一致**：`DropTarget`、`CategoryDrag`、`CategoryDragPhase`、`ReleaseAction`、`Tick` 在 Task 1–3 定义，Task 3–4 使用；`Message::{CategoryRowPress, CategoryDragOver, CategoryDragRelease}` 在 Task 2 定义，Task 3（拦截）、Task 4（发出）、Task 5 不再改动。
- **已知需要以编译器为准的点**：`crate::app::tree_drag_past_threshold` 的可达路径（定义在 `app/layout.rs`）、`set_category_drag_over` 的借用拆分、`CategorySelect` 的副作用抽取（Plan 1 之后 `clear_search` 已删除）、macOS 原生菜单里分隔线整理——均在对应步骤标明，行为保持不变。
- **只能在 GUI 里验证的点**：展开箭头的按下是否被捕获（Review Focus 2）、`MouseArea::on_move` 在按钮上是否照常触发——Task 4 Step 5 逐项人工验收；**未做 GUI 验收**（本环境无显示环境），见 Task 4 Step 5 顶部说明。
- **收尾状态（2026-10-02）**：Task 0–6 全部完成，提交于分支 `feat/todo-category-drag`（`ef476459` → `1bbe6537`，含 spec 更新 `457c00f2`）；`cargo fmt --check` 通过，clippy 14/16 警告与 main 基线持平，workspace 测试仅剩已知基线失败（`delete_confirm_spec_reflects_pending_target` + 3 个 `summary_pipeline`），`check-log-scope.sh` ok。合并到 main 由用户决定。
