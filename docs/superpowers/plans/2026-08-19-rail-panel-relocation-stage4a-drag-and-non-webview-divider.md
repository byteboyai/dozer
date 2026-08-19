# workspace 图标栏面板拖拽换栏 · Stage 4a:拖拽换栏机制 + 非 webview 面板分割线镜像 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现真正可用的拖拽换栏交互(图标栏按钮可以拖到另一条栏,也可以
在同栏内重新排序),并让 5 个没有原生 webview 的镜像面板(`Todo`/`Ssh`/
`GitLog`/`Agent`/`Conversations`)的内部分割线拖拽在面板被拖到非默认栏后
依然算得对(位置基准、拖拽方向都要跟着面板当前所在的栏与是否镜像走)。
**`Files`/`Project`/`Web` 三个挂了原生 webview 的面板不在这个 Stage
——它们的镜像 webview bounds 是 Stage 4b 的范围,理由见
`2026-08-19-rail-panel-drag-relocation-design.md` 背景一节。**

这是 4 个 Stage 里第一个能在 GUI 上真正观察到"面板换栏"和"内部渲染
顺序镜像"效果的阶段(Stage 1-3 因为没有拖拽,`RailLayout` 一直锁定默认
值,GUI 上看不出差异)。

**Architecture:** 拖拽机制照抄现有 `TabDrag` 的成熟手法(`App.tab_drag:
Option<TabDrag>`、按下即"武装"拖拽态、`on_move` 持续上报悬停位置、
`winit` 原生 `MouseInput{Released}` 收尾),新增一组独立的 `RailDrag`/
`Message::RailDragMove`/`RailDragEnd`,不复用 `TabDrag`/`TabGroup`(见
spec"拖拽:同栏重排 + 跨栏移动"一节的理由)。`apply_column_drag` 现在
按 `Divider` variant 硬编码"这条分割线一定在左区"或"一定在右区"、
"哪个子元素渲染在前"——这次改成从 `state.layout.rail_layout` 查这条
分割线对应面板**当前**在哪条栏、是否镜像,动态选基准坐标和 ratio 写入
方向。

**Tech Stack:** Rust 2024,复用 Stage 1-3 已落地的 `PanelKind`/
`RailLayout`/`Side`/`App::panel_mirrored`。

**Spec:** `docs/superpowers/specs/2026-08-19-rail-panel-drag-relocation-design.md`

## Global Constraints

- **前提:Stage 1、Stage 2、Stage 3 均已合并到 main。** 开工前确认
  `RailLayout::side_of`/`App::panel_mirrored`/`PanelKind::default_side`
  均已存在,`icon_rail(app, side)` 统一渲染函数已落地(Stage 2)。
- **独立分支开发,不直接提交 main。** 在新分支(如
  `feature/rail-panel-relocation-stage4a`)上完成全部 7 个 Task,提请
  审阅、通过后再合并回 `main`。每次 `git commit`/`git add` 前先跑
  `git branch --show-current`;开工前 `git status` 确认干净,发现不
  相关的改动要 `git stash push -- <具体路径>`(不要用 `-u`——这份计划
  文件本身、以及此前 Stage 2/3 的计划文件,都因为这个坑在写作过程中
  被误伤过,执行阶段同样要小心:开工前如果发现别人的未提交改动,只
  stash 那些具体路径,不要用 `-u` 图省事)。
- **每个 Task 结束都要求 `cargo build` 通过。**
- **`Files`/`Project`/`Web` 三个 webview 面板这个 Stage 里保持"能被拖到
  另一栏、但 webview 位置暂时不会跟着镜像"的状态**——这是预期中的过渡态
  (Stage 4b 才补 webview 镜像 bounds),不是 bug。这三个面板拖拽换栏后
  纯 iced 渲染部分(如果它们也有内部两栏,`Files`/`Project` 有、`Web`
  仅收藏夹展开时有)一样会走 Stage 3 已经做好的镜像逻辑,只是叠加的
  webview 子视图这个 Stage 里还按老坐标摆,会跟 iced 部分错位——GUI
  核对时看到这个错位不用管,已经记在这个 Stage 的"排期备注"里,Stage
  4b 解决。
- **`apply_row_drag`(`GitLogFileDiffSplit` 纵向分割)不改。** 它只依赖
  窗口高度,不依赖面板在左栏还是右栏,没有镜像意义(spec 已经说明)。

---

### Task 1: `RailDrag` 数据结构 + `rail_drag_move`/`end_rail_drag` 逻辑

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces: `pub struct RailDrag { pub source_side: Side, pub source_index:
  usize, pub pending_cross_side: Option<(Side, usize)> }`、
  `App.rail_drag: Option<RailDrag>`、
  `App::rail_drag_move(&mut self, side: Side, to: usize)`、
  `App::end_rail_drag(&mut self)`、`App::dragging_rail(&self) -> bool`

- [ ] **Step 1: 新增 `RailDrag` 结构体**(放在 `TabDrag` 定义附近)

```rust
/// 正在进行的图标栏面板拖拽(同栏重排 / 跨栏移动)。语义、生命周期管理
/// 手法照抄 `TabDrag`,但不复用它——`TabDrag`/`TabGroup` 是"同组内换位",
/// 图标栏这次还要支持"跨栏移动",合并进同一个类型会让校验逻辑变复杂。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailDrag {
    pub source_side: Side,
    pub source_index: usize,
    /// 悬停到另一栏时记录目标位置;`RailDragEnd` 才真正提交搬移,悬停
    /// 期间不搬、不落盘。悬停回源栏(或还没悬停到任何另一栏位置)时是
    /// `None`。
    pub pending_cross_side: Option<(Side, usize)>,
}
```

- [ ] **Step 2: `App` 新增 `rail_drag` 字段**

在 `App` 结构体里(`tab_drag: Option<TabDrag>` 附近)追加:

```rust
    rail_drag: Option<RailDrag>,
```

`App` 的初始化处(`tab_drag: None,` 附近)追加:

```rust
            rail_drag: None,
```

- [ ] **Step 3: `rail_drag_move`/`end_rail_drag`/`dragging_rail` 方法**

```rust
    /// 图标栏按钮拖拽悬停到 `side` 栏的第 `to` 个位置。同栏内是重排
    /// (立即生效,`Vec::remove`+`insert`);跨栏只记悬停目标,交给
    /// `end_rail_drag` 统一提交——避免每帧 `CursorMoved` 都触发一次
    /// `Vec` 搬移和后续的 `layout::save`。
    fn rail_drag_move(&mut self, side: Side, to: usize) {
        let Some(drag) = self.rail_drag else {
            return;
        };
        if drag.source_side == side {
            let panels = self.rail_layout.side_mut(side);
            let from = drag.source_index;
            if from == to || from >= panels.len() || to >= panels.len() {
                return;
            }
            let kind = panels.remove(from);
            panels.insert(to, kind);
            self.rail_drag = Some(RailDrag {
                source_side: side,
                source_index: to,
                pending_cross_side: None,
            });
        } else if let Some(drag) = &mut self.rail_drag {
            drag.pending_cross_side = Some((side, to));
        }
    }

    /// 结束图标栏拖拽:若悬停过另一栏,提交跨栏移动;否则(纯同栏重排,
    /// 或跨栏悬停后又移回源栏)不做搬移,只清拖拽态。跨栏移动会导致源栏
    /// 清空时整体 no-op(不支持"栏清空",见 spec 非目标)——这种情况下
    /// `RailLayout` 不变,不落盘。
    fn end_rail_drag(&mut self) {
        let Some(drag) = self.rail_drag.take() else {
            return;
        };
        let Some((target_side, target_index)) = drag.pending_cross_side else {
            return;
        };
        let source_panels = self.rail_layout.side(drag.source_side);
        if source_panels.len() <= 1 {
            // 源栏只剩这一个面板,搬走会清空——挡住,状态已经在上面
            // `take()` 时清空,这里直接返回即可,`RailLayout` 未改动。
            return;
        }
        let kind = self.rail_layout.side_mut(drag.source_side).remove(drag.source_index);
        let target_index = target_index.min(self.rail_layout.side(target_side).len());
        self.rail_layout.side_mut(target_side).insert(target_index, kind);
        // 被移动面板成为目标栏新 active,跟随"移动后在按钮所在一侧打开
        // 面板"的要求。
        match target_side {
            Side::Left => {
                self.left_view = kind;
                self.left_collapsed = false;
            }
            Side::Right => {
                self.right_view = kind;
                self.right_collapsed = false;
            }
        }
        layout::save(&self.layout_snapshot()).ok();
    }

    /// 当前是否正按住某个图标栏按钮(渲染侧据此把光标改成"抓取"把手,
    /// 同 `dragging_group` 对 `TabDrag` 的用法)。
    pub fn dragging_rail(&self) -> bool {
        self.rail_drag.is_some()
    }
```

**`layout_snapshot()`/`layout::save` 的确切调用方式请对照现有
`persist_open_projects`/`on_shell_layout_changed` 里"把当前状态存盘"的
既有写法,不要新起一套存盘路径**——如果现有存盘函数签名和这里假设的不
一致(比如需要先构造一个 `ShellLayout` 而不是直接传 `&self`),照现有
写法改,这里的示例代码按接口意图给,不代表最终一字不差的签名。

- [ ] **Step 4: 追加测试**

```rust
#[cfg(test)]
mod rail_drag_tests {
    use super::*;

    #[test]
    fn same_side_reorder_moves_kind_without_changing_active() {
        let mut app = App::default();
        app.rail_drag = Some(RailDrag {
            source_side: Side::Left,
            source_index: 0,
            pending_cross_side: None,
        });
        let before_active = app.left_view;
        app.rail_drag_move(Side::Left, 2);
        assert_ne!(app.rail_layout.left[0], app.rail_layout.left[2]);
        assert_eq!(app.left_view, before_active, "同栏重排不改变 active");
    }

    #[test]
    fn cross_side_move_relocates_and_activates_on_target_side() {
        let mut app = App::default();
        let kind = app.rail_layout.left[0];
        app.rail_drag = Some(RailDrag {
            source_side: Side::Left,
            source_index: 0,
            pending_cross_side: None,
        });
        app.rail_drag_move(Side::Right, 1);
        app.end_rail_drag();
        assert!(!app.rail_layout.left.contains(&kind));
        assert!(app.rail_layout.right.contains(&kind));
        assert_eq!(app.right_view, kind);
    }

    #[test]
    fn cross_side_move_is_noop_when_source_side_would_become_empty() {
        let mut app = App::default();
        // 把右栏清到只剩 1 个,验证"再搬最后一个"被挡住。
        while app.rail_layout.right.len() > 1 {
            let kind = app.rail_layout.right.remove(0);
            app.rail_layout.left.push(kind);
        }
        let only_kind = app.rail_layout.right[0];
        let before = app.rail_layout.clone();
        app.rail_drag = Some(RailDrag {
            source_side: Side::Right,
            source_index: 0,
            pending_cross_side: None,
        });
        app.rail_drag_move(Side::Left, 0);
        app.end_rail_drag();
        assert_eq!(app.rail_layout, before, "源栏只剩 1 个时禁止搬走,RailLayout 不变");
        assert_eq!(app.rail_layout.right, vec![only_kind]);
    }

    #[test]
    fn hovering_back_to_source_side_cancels_the_pending_cross_move() {
        let mut app = App::default();
        let before = app.rail_layout.clone();
        app.rail_drag = Some(RailDrag {
            source_side: Side::Left,
            source_index: 0,
            pending_cross_side: None,
        });
        app.rail_drag_move(Side::Right, 0); // 悬停到对侧
        app.rail_drag_move(Side::Left, 0); // 又悬停回源栏同一位置(no-op,index 没变)
        app.end_rail_drag();
        assert_eq!(app.rail_layout, before);
    }
}
```

（如果 `App` 没有可直接用的 `Default`/公开构造方式,改用现有测试里
构造 `App` 测试实例的既有辅助函数——参照 Stage 3 Task 1 遇到的同一个
问题,不要在这个 Task 里新起一套构造逻辑。）

- [ ] **Step 5: 编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer rail_drag`
Expected: 编译成功,4 个新测试通过

- [ ] **Step 6: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 新增 RailDrag 数据结构与同栏重排/跨栏移动逻辑"
```

---

### Task 2: `Message::RailDragMove`/`RailDragEnd` + 在 `panel_select` 里武装拖拽态

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces: `Message::RailDragMove { side: Side, index: usize }`、
  `Message::RailDragEnd`

- [ ] **Step 1: 新增 `Message` 变体**(`TabDragMove`/`TabDragEnd` 附近)

```rust
    RailDragMove { side: Side, index: usize },
    RailDragEnd,
```

- [ ] **Step 2: `update()` 分派**

```rust
            Message::RailDragMove { side, index } => self.rail_drag_move(side, index),
            Message::RailDragEnd => self.end_rail_drag(),
```

- [ ] **Step 3: 在 `panel_select` 里武装拖拽态**

`panel_select`(Stage 2 已经把 `left_icon_select`/`right_icon_select`
合并成这一个函数)开头追加(在原有逻辑之前,不影响原有的选中/收起
判断):

```rust
    fn panel_select(&mut self, kind: PanelKind) {
        let side = self.rail_layout.side_of(kind);
        let source_index = self
            .rail_layout
            .side(side)
            .iter()
            .position(|&k| k == kind)
            .expect("kind 应该在 side_of 返回的那一侧里,RailLayout 不变式已在 sanitize 里保证");
        self.rail_drag = Some(RailDrag {
            source_side: side,
            source_index,
            pending_cross_side: None,
        });
        // ... 原有的选中/收起/切入动作逻辑不变 ...
```

**这个 Task 只加"武装"这一小段,不改 `panel_select` 原有逻辑的其余部分
——原有逻辑(Stage 2 落地的)照原样保留在这段新代码之后。**

- [ ] **Step 4: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 新增 RailDragMove/RailDragEnd 消息,panel_select 时武装拖拽态"
```

---

### Task 3: 图标栏按钮接入拖拽感应层

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: Task 1-2
- Produces: `fn rail_drag_surface(...)`(仿 `tab_drag_surface`),
  `icon_rail`(Stage 2)循环体改用它包一层

- [ ] **Step 1: 新增 `rail_drag_surface`**(仿现有 `tab_drag_surface`,
  放在它附近)

```rust
/// 给一个图标栏按钮包上"拖拽换栏/换位"的感应层,手法同 `tab_drag_surface`
/// ——内容本身仍是原来的交互(点击选中在内部,见 `panel_select` 已经在
/// `Message::PanelSelect` 处理里武装拖拽态),外层只补 `on_move`:光标
/// 移动到这个按钮上时,若正在拖拽(`App::dragging_rail()`),上报
/// `RailDragMove { side, index }`。
fn rail_drag_surface(
    content: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>,
    side: Side,
    index: usize,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    MouseArea::new(content)
        .on_move(move |_| Message::RailDragMove { side, index })
        .into()
}
```

- [ ] **Step 2: `icon_rail`(Stage 2)循环体接入**

找到(Stage 2 落地的 `icon_rail` 函数内部,`for &kind in app.rail_layout
.side(side)` 循环里):

```rust
        let entry = match panel_badge(app, kind) {
            Some(badge) => stack![base, badge].into(),
            None => base,
        };
        content = content.push(entry);
```

改成:

```rust
        let entry = match panel_badge(app, kind) {
            Some(badge) => stack![base, badge].into(),
            None => base,
        };
        content = content.push(rail_drag_surface(entry, side, idx));
```

(`idx` 是循环变量——如果 Stage 2 落地时循环写的是 `for &kind in ...`
没有 `.enumerate()`,这里要改成 `for (idx, &kind) in app.rail_layout
.side(side).iter().enumerate()`,同步调整上面用到 `kind` 的地方不受
影响,只是多了 `idx` 可用。)

- [ ] **Step 3: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 图标栏按钮接入拖拽感应层"
```

---

### Task 4: `main.rs` 接入 `RailDragEnd`

**Files:**
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: `App::dragging_rail()`(Task 1)

- [ ] **Step 1: 追加 `MouseInput{Released}` 分支**

找到现有 `TabDragEnd` 那个分支:

```rust
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: winit::event::MouseButton::Left,
                    ..
                } if app.dragging_tab().is_some() => {
                    app.update(Message::TabDragEnd);
                    window.request_redraw();
                }
```

紧接着追加一个同构分支:

```rust
                // 图标栏拖拽换栏同理:左键松开即结束,换栏/换位是靠被拖
                // 过按钮的 on_move 驱动的,这里只负责收尾。
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: winit::event::MouseButton::Left,
                    ..
                } if app.dragging_rail() => {
                    app.update(Message::RailDragEnd);
                    window.request_redraw();
                }
```

- [ ] **Step 2: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): main.rs 接入 RailDragEnd(左键松开收尾)"
```

---

### Task 5: `apply_column_drag` side/镜像感知改造——`SshSplit`/`TodoSplit`/`GitLogSplit`

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `RailLayout::side_of`、`PanelKind::default_side`、
  `App::panel_mirrored` 的等价纯函数版本(见下)

**这三个面板默认都在左栏、默认"列表在前"**(同 Stage 3 摸底结论)。

- [ ] **Step 1: 新增辅助函数**(放在 `apply_column_drag` 附近)

```rust
/// 给定面板当前所在栏(不是默认栏,是"当前"——`RailLayout` 实时查),
/// 算出这条分割线要用哪个 zone 的横向基准(x0)与可分配宽度。左栏基准是
/// `icon_rail_width()`(从窗口左沿量),右栏基准是"窗口宽 - 右图标栏宽 -
/// 右区宽"(从窗口左沿量到右区左边界,同现有 `RightPairSplit` 分支已经
/// 在用的 `right_x0` 算法,这里把它提出来给两侧共用)。
fn pair_x0_and_width(side: Side, window_width: f32, state: &ShellState) -> (f32, f32) {
    match side {
        Side::Left => (
            byteui::theme::geometry::icon_rail_width(),
            pair_content_width(left_zone_width(window_width, state)),
        ),
        Side::Right => {
            let right_w = right_zone_width(window_width, state);
            (
                window_width - byteui::theme::geometry::icon_rail_width() - right_w,
                pair_content_width(right_w),
            )
        }
    }
}

/// 给定面板默认(未镜像)态下"列表侧是否渲染在前(pair 内第一个元素,
/// 几何上更靠左)"与当前是否处于镜像态,算出"列表侧现在是否渲染在前"。
/// `apply_column_drag` 算出的 `ratio` 恒是"pair 内第一个元素的宽度占比"
/// (鼠标左侧的宽度 / pair 总宽)——只有列表侧现在确实渲染在前时,
/// `ratio` 才能直接当"列表侧占比"写回 split 字段;渲染在后时要写
/// `1.0 - ratio`。
fn list_rendered_first(default_list_first: bool, mirrored: bool) -> bool {
    default_list_first != mirrored
}
```

- [ ] **Step 2: 改写 `Divider::SshSplit`**

找到:

```rust
        Divider::SshSplit => {
            let pair_w = pair_content_width(left_zone_width(window_width, &state));
            if pair_w <= 0.0 {
                return state.dims;
            }
            let ratio = ((logical_x - byteui::theme::geometry::icon_rail_width()) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            PanelDims {
                ssh_split: ratio,
                ..state.dims
            }
        }
```

改成:

```rust
        Divider::SshSplit => {
            let side = state.layout.rail_layout.side_of(PanelKind::Ssh);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != PanelKind::Ssh.default_side();
            let ratio = if list_rendered_first(true, mirrored) {
                raw_ratio
            } else {
                1.0 - raw_ratio
            };
            PanelDims {
                ssh_split: ratio,
                ..state.dims
            }
        }
```

- [ ] **Step 3: 同样改写 `Divider::TodoSplit`**(把 `PanelKind::Ssh`/
  `ssh_split` 换成 `PanelKind::Todo`/`todo_split`,其余结构一字不动)

- [ ] **Step 4: 同样改写 `Divider::GitLogSplit`**(把 `PanelKind::Ssh`/
  `ssh_split` 换成 `PanelKind::GitLog`/`git_log_split`,其余结构一字
  不动)

- [ ] **Step 5: 追加测试**——复用文件里已有的 `test_state()` 辅助函数
  (`ShellState { layout: ShellLayout::default(), dims: PanelDims::
  default(), left_view: PanelKind::Files, right_view: PanelKind::Agent,
  .. }`,`layout.rail_layout` 已经是 `RailLayout::default()`,Ssh/Todo/
  GitLog 都在左栏)。防回归 + 方向性断言两组,照抄文件里现有
  `apply_column_drag_browser_bookmarks_split_direction_matches_content_
  side` 那条"near/far 比较"的手法,不手算精确数值(那条既有测试就是
  这么做的,不是这次新引入的风格)。

```rust
#[cfg(test)]
mod apply_column_drag_ssh_todo_gitlog_mirror_tests {
    use super::*;

    #[test]
    fn ssh_split_direction_on_default_side_matches_pre_migration_behavior() {
        // 默认栏(左)下,Ssh 列表侧渲染在前(左)——拖拽点越靠右,
        // 列表占比应该越大,和改造前的固定行为(`icon_rail_width()` 为
        // 基准直接写 ratio,不翻转)完全一致,这条测试是防回归锚。
        let state = test_state();
        let window_width = 1600.0;
        let near = apply_column_drag(state.clone(), Divider::SshSplit, window_width, 300.0);
        let far = apply_column_drag(state, Divider::SshSplit, window_width, 500.0);
        assert!(
            far.ssh_split > near.ssh_split,
            "near={} far={}",
            near.ssh_split,
            far.ssh_split
        );
    }

    #[test]
    fn ssh_split_direction_flips_when_relocated_to_right_side() {
        // 把 Ssh 挪到右栏(非默认栏,`panel_mirrored` 应为 true,内部
        // 渲染顺序镜像成"列表在后")。同样"拖拽点越靠右"的相对移动,
        // 这次列表占比应该越*小*——和上面那条默认栏的方向相反,这才是
        // 这个 Task 真正要验证的新行为。
        let mut state = test_state();
        state.layout.rail_layout.left.retain(|&k| k != PanelKind::Ssh);
        state.layout.rail_layout.right.push(PanelKind::Ssh);
        let window_width = 1600.0;
        // 拖拽点取值改成落在"右栏"的横向范围内(右栏在窗口右侧,
        // `pair_x0_and_width(Side::Right, ...)` 算出的 x0 远大于左栏的
        // `icon_rail_width()`),否则拖拽点落在图标栏或左栏范围内,
        // ratio 会被 clamp 死在边界,测不出方向性。
        let right_x0 = window_width - byteui::theme::geometry::icon_rail_width()
            - right_zone_width(window_width, &state);
        let near = apply_column_drag(
            state.clone(),
            Divider::SshSplit,
            window_width,
            right_x0 + 50.0,
        );
        let far = apply_column_drag(
            state,
            Divider::SshSplit,
            window_width,
            right_x0 + 250.0,
        );
        assert!(
            far.ssh_split < near.ssh_split,
            "镜像态下方向应反转:near={} far={}",
            near.ssh_split,
            far.ssh_split
        );
    }
}
```

`Todo`/`GitLog` 各自照抄这两条测试(换 `Divider::SshSplit`→
`Divider::TodoSplit`/`Divider::GitLogSplit`、`ssh_split`→`todo_split`/
`git_log_split`),这个 Task 一共补 6 条测试(2 个面板 × 3 = 不对,是
3 个面板 × 2 条 = 6 条)。

- [ ] **Step 6: 编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer apply_column_drag`
Expected: 编译成功,新旧测试全部通过(改造前的 `SshSplit`/`TodoSplit`/
`GitLogSplit` 相关既有测试——如果有——必须在默认栏场景下逐字节保持
原有断言值,这是防回归的关键)

- [ ] **Step 7: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): apply_column_drag 的 Ssh/Todo/GitLog 分支改为 side+镜像感知"
```

---

### Task 6: `apply_column_drag` side/镜像感知改造——`RightPairSplit`(Agent/Conversations)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**默认在右栏、默认"内容在前"(和 Task 5 三个面板方向相反,Stage 3 已经
摸底确认过)。**

- [ ] **Step 1: 改写**

找到现有 `Divider::RightPairSplit` 分支(按 `state.right_view` 决定写
`agent_split` 还是 `conversations_split`,现有逻辑已经在做 `1.0 - ratio`
——这个 Task 要把"固定在右区、固定 1.0-ratio"改成"按当前所在栏选基准、
按当前是否镜像决定要不要再翻一次"):

```rust
        Divider::RightPairSplit => {
            let kind = state.right_view; // Agent 或 Conversations
            let side = state.layout.rail_layout.side_of(kind);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != kind.default_side();
            // Agent/Conversations 默认"内容在前"(default_list_first = false),
            // 和 Task 5 三个面板相反。
            let ratio = if list_rendered_first(false, mirrored) {
                raw_ratio
            } else {
                1.0 - raw_ratio
            };
            match kind {
                PanelKind::Agent => PanelDims {
                    agent_split: ratio,
                    ..state.dims
                },
                PanelKind::Conversations => PanelDims {
                    conversations_split: ratio,
                    ..state.dims
                },
                PanelKind::Usage => state.dims,
                PanelKind::Acceptance => state.dims,
                _ => unreachable!(
                    "RightPairSplit 只会在 state.right_view 是 Agent/Conversations/\
                     Usage/Acceptance 之一时触发——Stage 1 遗留的兜底,这里维持"
                ),
            }
        }
```

**这里删掉了原来"两者互补,写 1.0-ratio"那条注释里描述的固定翻转
——现在翻不翻由 `list_rendered_first` 动态判断,默认栏(`mirrored =
false`)下 `list_rendered_first(false, false) = false`,依然走
`1.0 - raw_ratio` 这条分支,和原来的固定行为完全一致,不是行为回归。**

- [ ] **Step 2: 追加测试**——同 Task 5 的 near/far 方向性手法,但注意
  `test_state()` 默认 `right_view: PanelKind::Agent`,测 `Conversations`
  时需要先把 `state.right_view` 改成 `PanelKind::Conversations`(否则
  `apply_column_drag` 走的是 `kind = state.right_view` 取到 `Agent`,
  测试实际验证的还是 Agent 那条分支)。

```rust
#[cfg(test)]
mod apply_column_drag_right_pair_mirror_tests {
    use super::*;

    #[test]
    fn agent_split_direction_on_default_side_matches_pre_migration_behavior() {
        // Agent 默认在右栏、默认"内容(终端)在前",拖拽点越靠右,
        // 内容占比越大 → agent_split(列表占比)越*小*,和改造前
        // "写 1.0-ratio"的固定行为一致,防回归锚。
        let state = test_state(); // right_view 已经是 PanelKind::Agent
        let window_width = 1600.0;
        let right_x0 = window_width - byteui::theme::geometry::icon_rail_width()
            - right_zone_width(window_width, &state);
        let near = apply_column_drag(
            state.clone(),
            Divider::RightPairSplit,
            window_width,
            right_x0 + 50.0,
        );
        let far = apply_column_drag(
            state,
            Divider::RightPairSplit,
            window_width,
            right_x0 + 250.0,
        );
        assert!(
            far.agent_split < near.agent_split,
            "near={} far={}",
            near.agent_split,
            far.agent_split
        );
    }

    #[test]
    fn agent_split_direction_flips_when_relocated_to_left_side() {
        // 把 Agent 挪到左栏(非默认栏)——镜像后"列表在前",方向应该
        // 反转:拖拽点越靠右,列表占比应该越*大*。
        let mut state = test_state();
        state.layout.rail_layout.right.retain(|&k| k != PanelKind::Agent);
        state.layout.rail_layout.left.push(PanelKind::Agent);
        let window_width = 1600.0;
        let near = apply_column_drag(
            state.clone(),
            Divider::RightPairSplit,
            window_width,
            byteui::theme::geometry::icon_rail_width() + 50.0,
        );
        let far = apply_column_drag(
            state,
            Divider::RightPairSplit,
            window_width,
            byteui::theme::geometry::icon_rail_width() + 250.0,
        );
        assert!(
            far.agent_split > near.agent_split,
            "镜像态下方向应反转:near={} far={}",
            near.agent_split,
            far.agent_split
        );
    }

    #[test]
    fn conversations_split_direction_on_default_side_matches_pre_migration_behavior() {
        let mut state = test_state();
        state.right_view = PanelKind::Conversations;
        let window_width = 1600.0;
        let right_x0 = window_width - byteui::theme::geometry::icon_rail_width()
            - right_zone_width(window_width, &state);
        let near = apply_column_drag(
            state.clone(),
            Divider::RightPairSplit,
            window_width,
            right_x0 + 50.0,
        );
        let far = apply_column_drag(
            state,
            Divider::RightPairSplit,
            window_width,
            right_x0 + 250.0,
        );
        assert!(
            far.conversations_split < near.conversations_split,
            "near={} far={}",
            near.conversations_split,
            far.conversations_split
        );
    }

    #[test]
    fn conversations_split_direction_flips_when_relocated_to_left_side() {
        let mut state = test_state();
        state.right_view = PanelKind::Conversations;
        state.layout.rail_layout.right.retain(|&k| k != PanelKind::Conversations);
        state.layout.rail_layout.left.push(PanelKind::Conversations);
        let window_width = 1600.0;
        let near = apply_column_drag(
            state.clone(),
            Divider::RightPairSplit,
            window_width,
            byteui::theme::geometry::icon_rail_width() + 50.0,
        );
        let far = apply_column_drag(
            state,
            Divider::RightPairSplit,
            window_width,
            byteui::theme::geometry::icon_rail_width() + 250.0,
        );
        assert!(
            far.conversations_split > near.conversations_split,
            "镜像态下方向应反转:near={} far={}",
            near.conversations_split,
            far.conversations_split
        );
    }
}
```

- [ ] **Step 3: 编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer apply_column_drag`
Expected: 编译成功,全部通过

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): apply_column_drag 的 RightPairSplit 分支改为 side+镜像感知"
```

---

### Task 7: 全量验证(这个 Stage 第一次能在 GUI 上真正看到拖拽 + 镜像)

**Files:**
- 无修改,纯验证

**Interfaces:**
- Consumes: Task 1-6 已完成

- [ ] **Step 1: 全量编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer`
Expected: 编译成功;测试全部通过(数量比 Stage 3 结束时的基线多出
Task 1/5/6 新增的测试)。

- [ ] **Step 2: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 无新增警告、无格式差异。

- [ ] **Step 3: GUI 拖拽核对**

构建一个独立命名的临时二进制,启动后逐条核对:

- **同栏重排**:在左图标栏内把两个图标顺序拖换,松手后顺序生效、
  `active` 不变。右栏同理。
- **跨栏移动(非 webview 面板)**:把 `Todo`/`Ssh`/`GitLog` 之一从左栏
  拖到右栏,松手后:该面板自动在右栏展开并选中,左栏收起态正确退回
  相邻面板;面板内部渲染顺序确实镜像了(比如 Todo 的侧边栏/内容顺序
  反过来)。把 `Agent`/`Conversations` 之一从右栏拖到左栏,同样核对。
- **分割线拖拽方向在镜像态下正确**:上面任选一个已经挪到对侧的面板,
  拖动它内部的分割线,确认拖拽手感方向正常(往右拖列表变宽还是变窄,
  应该和它镜像前"往左拖"的手感对称,不应该出现"往右拖结果列表越拖
  越靠边、分割线冲出可视区"这类错乱)。
- **不支持栏清空**:把右栏 4 个面板依次全部拖到左栏,确认最后一次
  (右栏只剩 1 个时)被正确挡住,右栏保留最后 1 个,图标栏不会变空。
- **`Files`/`Project`/`Web` 三个 webview 面板允许被拖到对侧,但 webview
  子视图位置这个 Stage 里预期会和 iced 部分错位**(不算 bug,Stage 4b
  解决)——核对它们至少不会崩溃、iced 部分(有内部两栏的话)确实镜像了,
  webview 错位现象记录下来留给 Stage 4b 对照验证用,不需要在这个 Stage
  修。
- **退出重开**:核对 `RailLayout`(换栏结果)、`left_view`/`right_view`
  (哪个面板当前 active)、`left_collapsed`/`right_collapsed` 跨重启
  保留。

完成后关闭该临时实例、删除临时二进制,不留后台进程。

- [ ] **Step 4: Commit(如果 Step 1-3 发现并修复了任何问题)**

```bash
git branch --show-current
git add -A
git commit -m "fix: Stage 4a 全量验证发现的问题修复"
```

(如果全部一次通过,跳过,不产生空 commit。)

---

## 完工验收

1. `git log --oneline` 确认全部 commit 都在当前分支上,没有漂到 `main`。
2. `cargo build`/`cargo test`/`cargo clippy`/`cargo fmt --check` 全绿。
3. GUI 核对:5 个非 webview 镜像面板的拖拽换栏、内部渲染顺序镜像、
   分割线拖拽方向,三者组合起来行为正确;不支持栏清空的规则生效;
   跨重启保留验证通过。
4. 提请审阅。审阅通过合并后,Stage 4b(`Files`/`Project`/`Web` 三个
   webview 面板的镜像 bounds + 它们各自内部分割线的 side/镜像感知,
   补齐这个 Stage 里刻意跳过的部分)才能开工——这是整个"图标栏面板
   拖拽换栏"项目最后一个 Stage。

## 排期备注

- **`Files`/`Project`/`Web` 拖到对侧后 webview 子视图位置错位**是这个
  Stage 明确接受的过渡态,Stage 4b 专门解决(镜像版 `preview_content_
  bounds`/`browser_content_bounds`/project 预览 bounds 三个函数,以及
  `Divider::LeftPairSplit`/`ProjectSplit`/`BrowserBookmarksSplit` 三个
  分割线同样需要这个 Stage 已经做出来的 `pair_x0_and_width`/
  `list_rendered_first` 两个辅助函数——Stage 4b 直接复用,不用重新
  设计一套)。
- 这个 Stage 结束、Stage 4b 开工前,建议先做一轮"这三个 webview 面板
  暂时先别让用户拖到对侧"的产品决策确认(比如图标栏拖拽时对这三个
  按钮加一个"暂不支持跨栏"的视觉提示或直接禁用跨栏放下)——当前设计
  是"能拖但视觉错位",这在两个 Stage 之间的窗口期内如果这个分支真的
  发布给用户用,体验是有缺陷的。这个决策不在这份计划的范围内,留给
  开工 Stage 4b 之前由用户拍板:是"两个 Stage 背靠背做完再发布"还是
  "Stage 4a 先临时禁用这三个面板的跨栏拖放,Stage 4b 再放开"。
