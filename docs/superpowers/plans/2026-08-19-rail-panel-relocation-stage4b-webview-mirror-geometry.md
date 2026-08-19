# workspace 图标栏面板拖拽换栏 · Stage 4b:webview 面板镜像几何(收尾)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `Files`/`Project`/`Web` 三个挂了原生 `wry` webview 的面板,拖到
非默认栏后,webview 子视图的像素位置要跟纯 iced 部分(Stage 3 已经做好
的镜像渲染)对齐,内部分割线拖拽方向也要正确(同 Stage 4a 对 5 个非
webview 面板做的事)。**这是整个"图标栏面板拖拽换栏"项目 4 个 Stage
里最后一个,也是风险最集中的一个**——原摸底就把这三个面板点名为"这次
改动风险最集中的地方"(webview 像素级摆位,不是 iced 声明式布局,算错
不会报错,只会肉眼可见地摆错位置或叠在别的内容上面)。

**Architecture:** 摸底(写这份计划前重读 `main` 当前代码)发现三个
函数都各自独立、用同一种"以左图标栏宽度为基准向右偏移、列表恒渲染
在前"的硬编码假设算 Files/Project/Web 的位置:

- `preview_content_bounds`(webview 矩形本身,`main.rs` 拿去摆 `wry`
  webview)——非放大态、放大态(`MaximizedPane::Left`)两条路径各自
  重复一遍这个假设。
- `left_files_tree_bounds`(Files 专属:文件树列表 Scrollable 的矩形,
  供外部拖拽命中测试换算"拖到第几行")——同样两条路径。
- `is_in_preview_column`(键盘焦点路由:点击落在预览列就把键盘交给
  webview)——同样两条路径,且 Files/Project/Web 三个面板都要判。

放大态(`MaximizedPane::Left`)本身**不需要**按"当前在左栏还是右栏"
选基准——`maximized_box_x_range` 已经是左右对称的同一个盒子(两条
图标栏都还露在外面),这次不用碰;放大态里仍然需要处理的是"面板内部
list/content 两列谁先渲染"这一层镜像,和非放大态是同一个问题,只是
外层盒子不同。

统一抽一个 `pair_columns(pair_w, split, mirrored) -> PairColumns`
辅助函数(相对 zone/盒子内容区左边界的偏移,不含 zone 自己的 x0)+
复用 Stage 4a 已经写好的 `pair_x0_and_width`(zone 级的 x0/宽度选择,
非放大态専用)、`list_rendered_first`(镜像方向判断),三个函数各自
只需要"决定 zone/盒子基准 + 调这两个辅助函数 + 拼最终矩形",不用各自
重新推一遍镜像数学。

**Tech Stack:** Rust 2024,复用 Stage 4a 的 `pair_x0_and_width`/
`list_rendered_first`。

**Spec:** `docs/superpowers/specs/2026-08-19-rail-panel-drag-relocation-design.md`

## Global Constraints

- **前提:Stage 1-4a 均已合并到 main。** 开工前确认 `pair_x0_and_width`/
  `list_rendered_first`(Stage 4a Task 5)、`RailDrag`/`Message::
  RailDragMove`/`RailDragEnd`(Stage 4a Task 1-4)均已存在。
- **独立分支开发,不直接提交 main。** 在新分支(如
  `feature/rail-panel-relocation-stage4b`)上完成全部 8 个 Task,提请
  审阅、通过后再合并回 `main`。每次 `git commit`/`git add` 前先跑
  `git branch --show-current`;开工前 `git status` 确认干净,发现不
  相关的改动要 `git stash push -- <具体路径>`(不要用 `-u`)。
- **每个 Task 结束都要求 `cargo build` 通过。**
- **这是整个项目最后一个 Stage。** Task 8(全量验证)的 GUI 核对范围
  覆盖全部 11 个面板、全部 4 个 Stage 累积的能力,不只是这个 Stage
  新增的部分——这是唯一一次"全项目收尾式"验证,之前 3 个 Stage 的
  验证范围都只覆盖各自 Stage 新增的部分。
- **像素级精度不强求完美,但方向和量级必须对**——webview 矩形算错一两
  像素不算这个 Stage 的失败(现状本来就有"不追求像素级精确"的既有
  容忍度,比如 `terminal_pane_pixel_size` 的文档注释),但如果 webview
  整块摆到窗口另一侧、或者叠在了不该叠的内容上面,就是这个 Stage 要
  堵住的真实 bug。

---

### Task 1: `pair_columns` 辅助函数(镜像感知的列内偏移计算)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `pair_list_content_width`(既有)
- Produces: `struct PairColumns { list_x: f32, list_w: f32, content_x: f32,
  content_w: f32 }`、`fn pair_columns(pair_w: f32, split: f32, mirrored:
  bool) -> PairColumns`

- [ ] **Step 1: 新增**(放在 `pair_list_content_width` 附近)

```rust
/// 配对视图内 list/content 两列相对**所在 zone/盒子内容区左边界**的
/// 横向偏移与宽度(不含 zone/盒子自己的 x0——调用方自己加)。`mirrored`
/// = false 时 list 在前(x=0)、content 在后(x=list_w+divider_width);
/// `mirrored` = true 时反过来。`preview_content_bounds`/
/// `left_files_tree_bounds`/`is_in_preview_column` 三个函数(webview
/// 矩形、文件树命中、焦点路由)都靠这一份算,不许各写各的偏移公式。
struct PairColumns {
    list_x: f32,
    list_w: f32,
    content_x: f32,
    content_w: f32,
}

fn pair_columns(pair_w: f32, split: f32, mirrored: bool) -> PairColumns {
    let (list_w, content_w) = pair_list_content_width(pair_w, split);
    let divider = byteui::theme::geometry::divider_width();
    if mirrored {
        PairColumns {
            content_x: 0.0,
            content_w,
            list_x: content_w + divider,
            list_w,
        }
    } else {
        PairColumns {
            list_x: 0.0,
            list_w,
            content_x: list_w + divider,
            content_w,
        }
    }
}
```

- [ ] **Step 2: 追加测试**

```rust
#[cfg(test)]
mod pair_columns_tests {
    use super::*;

    #[test]
    fn not_mirrored_puts_list_first() {
        let c = pair_columns(600.0, 0.4, false);
        assert_eq!(c.list_x, 0.0);
        assert!(c.content_x > c.list_x + c.list_w);
    }

    #[test]
    fn mirrored_puts_content_first() {
        let c = pair_columns(600.0, 0.4, true);
        assert_eq!(c.content_x, 0.0);
        assert!(c.list_x > c.content_x + c.content_w);
    }

    #[test]
    fn list_and_content_widths_sum_to_pair_width_regardless_of_mirror() {
        let pair_w = 600.0;
        let a = pair_columns(pair_w, 0.4, false);
        let b = pair_columns(pair_w, 0.4, true);
        assert!((a.list_w + a.content_w - pair_w).abs() < 0.01);
        assert_eq!(a.list_w, b.list_w);
        assert_eq!(a.content_w, b.content_w);
    }
}
```

- [ ] **Step 3: 编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer pair_columns`
Expected: 编译成功,3 个新测试通过

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 新增 pair_columns 镜像感知列偏移辅助函数"
```

---

### Task 2: `preview_content_bounds` 的 `Files` 分支(非放大态 + 放大态)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

- [ ] **Step 1: 改写放大态分支**

找到(`preview_content_bounds` 函数里,`if state.maximized == Some
(MaximizedPane::Left) { ... }` 块内):

```rust
            PanelKind::Files => {
                let y = y0 + byteui::theme::geometry::preview_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::preview_chrome_top_px() - 8.0).max(0.0);
                let pair_w = pair_content_width(avail_w);
                let (list_w, content_w) = pair_list_content_width(pair_w, state.dims.files_split);
                let x = x0 + list_w + byteui::theme::geometry::divider_width() + 8.0;
                let w = (content_w - 16.0).max(0.0);
                (x, y, w, h)
            }
```

改成:

```rust
            PanelKind::Files => {
                let y = y0 + byteui::theme::geometry::preview_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::preview_chrome_top_px() - 8.0).max(0.0);
                let pair_w = pair_content_width(avail_w);
                let mirrored = state.layout.rail_layout.side_of(PanelKind::Files)
                    != PanelKind::Files.default_side();
                let cols = pair_columns(pair_w, state.dims.files_split, mirrored);
                let x = x0 + cols.content_x + byteui::theme::geometry::divider_width() + 8.0;
                let w = (cols.content_w - 16.0).max(0.0);
                (x, y, w, h)
            }
```

**注意 `x` 的公式:非镜像态 `cols.content_x == list_w`,加上
`divider_width()+8.0` 和原公式一致;镜像态 `cols.content_x == 0.0`
(content 在最前),这时 `+divider_width()+8.0` 就不对了——content
紧贴 `x0`,不该再加分隔线宽度。把上面这行换成分两种情况:**

```rust
                let x = if mirrored {
                    x0 + cols.content_x + 8.0
                } else {
                    x0 + cols.content_x + byteui::theme::geometry::divider_width() + 8.0
                };
```

**这个"+divider_width()"只在 content 不是第一列时才需要(要跨过前面
那一列 + 分隔线本身),`pair_columns` 返回的 `content_x` 已经是"content
列左边界相对 zone/盒子内容区左边界"的偏移,不含 zone/盒子自己的
`8.0`(容器内边距)——`8.0` 这个字面量是这里所有分支共有的固定内边距,
不是 `pair_columns` 管的范围,原样保留在外层加。**

- [ ] **Step 2: 改写非放大态分支**

找到:

```rust
        PanelKind::Files => {
            let y = y_top(byteui::theme::geometry::preview_chrome_top_px());
            let h = h_for(y);
            let pair_w = pair_content_width(left_w);
            let (list_w, content_w) = pair_list_content_width(pair_w, state.dims.files_split);
            let x = byteui::theme::geometry::icon_rail_width()
                + list_w
                + byteui::theme::geometry::divider_width()
                + 8.0
                + m.left;
            let w = (content_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
```

改成:

```rust
        PanelKind::Files => {
            let side = state.layout.rail_layout.side_of(PanelKind::Files);
            let mirrored = side != PanelKind::Files.default_side();
            let (zone_x0, pair_w) = pair_x0_and_width(side, window_width, state);
            let y = y_top(byteui::theme::geometry::preview_chrome_top_px());
            let h = h_for(y);
            let cols = pair_columns(pair_w, state.dims.files_split, mirrored);
            let x = if mirrored {
                zone_x0 + cols.content_x + m.left
            } else {
                zone_x0 + cols.content_x + byteui::theme::geometry::divider_width() + 8.0 + m.left
            };
            let w = (cols.content_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
```

**这里 `m`(`theme::region::left_zone().margin`)这个 Task 先不改成
side 相关**——`m` 是函数级别的局部变量,在 Files/Web/Project 三个
分支之间共用,改成 side 相关需要同时改这三个分支,放在 Task 4(Web,
最后一个改的分支)里统一处理,避免中间状态三个分支各用不同的 `m`
定义方式导致来回改。这个 Task 先保持 `m` 引用不变(继续用外层已经
算好的 `left_zone().margin`),只把 `zone_x0`/`pair_w`/`x`/`w` 的核心
公式改对。

- [ ] **Step 3: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): preview_content_bounds 的 Files 分支改为 side+镜像感知"
```

---

### Task 3: `preview_content_bounds` 的 `Project` 分支(非放大态 + 放大态)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

同 Task 2 手法,`PanelKind::Files`/`files_split` 换成 `PanelKind::
Project`/`project_split`,其余结构一字不动(包括"镜像态不加
`divider_width()`"那条注意事项)。

- [ ] **Step 1: 改写放大态分支**

- [ ] **Step 2: 改写非放大态分支**

- [ ] **Step 3: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): preview_content_bounds 的 Project 分支改为 side+镜像感知"
```

---

### Task 4: `preview_content_bounds` 的 `Web` 分支(非放大态 + 放大态)+ 统一 `m` 为 side 相关

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**`Web` 单栏(无内部镜像,`bookmarks_open` 时例外——见下),但仍需要
side 感知**(挪到右栏后要用 `right_zone_width`/`right_zone().margin`,
不是 `left_zone_width`/`left_zone().margin`)。**这个 Task 顺便把
`m` 改成 side 相关**,回头把 Task 2/3 留的"`m` 暂不变"这个技术债还清
——三个分支现在都需要 side 相关的 `m`,一次性改比分三次改更不容易
漏改。

- [ ] **Step 1: 把函数里 `let m = theme::region::left_zone().margin;`
  这一行挪到每个分支内部按 side 取值**,不再在函数顶层算一次共用。

找到函数顶层的:

```rust
    let m = theme::region::left_zone().margin;
```

删除这一行(顶层不再统一算)。Task 2/3 已经改完的 Files/Project
分支内部,各自在算 `mirrored`/`side` 那几行旁边补一行:

```rust
            let m = match side {
                Side::Left => theme::region::left_zone().margin,
                Side::Right => theme::region::right_zone().margin,
            };
```

(回到 Task 2/3 改过的代码里补这一行——这个 Task 属于"收尾统一",
执行时需要一并回补 Task 2/3 落下的这半步,不是这个 Task 凭空多出
的范围外工作。)

- [ ] **Step 2: 改写放大态 `Web` 分支**

找到:

```rust
            PanelKind::Web => {
                let y = y0 + byteui::theme::geometry::browser_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::browser_chrome_top_px() - 8.0).max(0.0);
                let x = x0 + 8.0;
                let w = (avail_w - 16.0).max(0.0);
                (x, y, w, h)
            }
```

**放大态本身不需要改**(`x0`/`avail_w` 已经是左右对称的同一个盒子,
Web 单栏占满整个盒子,和面板在哪条栏无关)——这一步只是确认现状不用
动,不产出代码改动。

- [ ] **Step 3: 改写非放大态 `Web` 分支**

找到:

```rust
        PanelKind::Web => {
            let y = y_top(byteui::theme::geometry::browser_chrome_top_px());
            let h = h_for(y);
            let x = byteui::theme::geometry::icon_rail_width() + 8.0 + m.left;
            let w = if state.browser_bookmarks_open {
                let pair_w = pair_content_width(left_w);
                let (_bookmarks_w, content_w) =
                    pair_list_content_width(pair_w, 1.0 - state.dims.browser_bookmarks_split);
                (content_w - 16.0 - m.left - m.right).max(0.0)
            } else {
                (left_w - 16.0 - m.left - m.right).max(0.0)
            };
            (x, y, w, h)
        }
```

改成:

```rust
        PanelKind::Web => {
            let side = state.layout.rail_layout.side_of(PanelKind::Web);
            let mirrored = side != PanelKind::Web.default_side();
            let (zone_x0, zone_w) = match side {
                Side::Left => (
                    byteui::theme::geometry::icon_rail_width(),
                    left_zone_width(window_width, state),
                ),
                Side::Right => {
                    let right_w = right_zone_width(window_width, state);
                    (
                        window_width - byteui::theme::geometry::icon_rail_width() - right_w,
                        right_w,
                    )
                }
            };
            let m = match side {
                Side::Left => theme::region::left_zone().margin,
                Side::Right => theme::region::right_zone().margin,
            };
            let y = y_top(byteui::theme::geometry::browser_chrome_top_px());
            let h = h_for(y);
            let x = zone_x0 + 8.0 + m.left;
            let w = if state.browser_bookmarks_open {
                // 内容占比字段是 `browser_bookmarks_split` 本身(注意和
                // 其余 7 个 split 字段"存列表侧占比"的语义相反——见 spec
                // Web 那一行的备注),`pair_columns` 的 `split` 参数吃的
                // 是"内容占比"还是"列表占比"取决于调用方怎么对应
                // `list`/`content` 概念:这里把"内容"当 `pair_columns`
                // 的 `content`,所以要传 `1.0 - browser_bookmarks_split`
                // 当"list(收藏夹)占比",维持和改造前 `1.0 -
                // browser_bookmarks_split` 这条既有算式的字面对应关系。
                let pair_w = pair_content_width(zone_w);
                let cols = pair_columns(pair_w, 1.0 - state.dims.browser_bookmarks_split, mirrored);
                (cols.content_w - 16.0 - m.left - m.right).max(0.0)
            } else {
                (zone_w - 16.0 - m.left - m.right).max(0.0)
            };
            (x, y, w, h)
        }
```

**`bookmarks_open` 为 `false` 时(单栏)`mirrored` 不起作用,这是
预期的 no-op,不是遗漏——同 Stage 3 Task 9 对 Web 面板的处理保持一致。**

- [ ] **Step 4: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): preview_content_bounds 的 Web 分支改为 side 感知,统一三分支 margin 取值"
```

---

### Task 5: `left_files_tree_bounds`(Files 专属,非放大态 + 放大态)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

- [ ] **Step 1: 改写放大态路径**

找到:

```rust
    if state.maximized == Some(MaximizedPane::Left) {
        let (x0, avail_w) = maximized_box_x_range(window_width);
        let y_top = byteui::theme::geometry::top_bar_height()
            + byteui::theme::geometry::maximize_overlay_padding();
        let avail_h = (maximized_box_height(window_height)
            - byteui::theme::geometry::status_bar_height())
        .max(0.0);
        let pair_w = pair_content_width(avail_w);
        let (list_w, _content_w) = pair_list_content_width(pair_w, state.dims.files_split);
        let x = x0 + m.left + p.padding.left;
        let w = (list_w - p.padding.left - p.padding.right).max(0.0);
        let y = y_top + m.top + p.padding.top + theme::geometry::tree_chrome_top_px();
        let h = (avail_h
            - m.top
            - m.bottom
            - p.padding.top
            - p.padding.bottom
            - theme::geometry::tree_chrome_top_px()
            - theme::geometry::tree_chrome_bottom_px())
        .max(0.0);
        return (x, y, w, h);
    }
```

改成:

```rust
    if state.maximized == Some(MaximizedPane::Left) {
        let mirrored =
            state.layout.rail_layout.side_of(PanelKind::Files) != PanelKind::Files.default_side();
        let (x0, avail_w) = maximized_box_x_range(window_width);
        let y_top = byteui::theme::geometry::top_bar_height()
            + byteui::theme::geometry::maximize_overlay_padding();
        let avail_h = (maximized_box_height(window_height)
            - byteui::theme::geometry::status_bar_height())
        .max(0.0);
        let pair_w = pair_content_width(avail_w);
        let cols = pair_columns(pair_w, state.dims.files_split, mirrored);
        let x = if mirrored {
            x0 + cols.list_x + byteui::theme::geometry::divider_width() + m.left + p.padding.left
        } else {
            x0 + cols.list_x + m.left + p.padding.left
        };
        let w = (cols.list_w - p.padding.left - p.padding.right).max(0.0);
        let y = y_top + m.top + p.padding.top + theme::geometry::tree_chrome_top_px();
        let h = (avail_h
            - m.top
            - m.bottom
            - p.padding.top
            - p.padding.bottom
            - theme::geometry::tree_chrome_top_px()
            - theme::geometry::tree_chrome_bottom_px())
        .max(0.0);
        return (x, y, w, h);
    }
```

**这里 `x` 的"镜像态要加 `divider_width()`"和 Task 2 里 content 分支
"非镜像态要加 `divider_width()`"刚好互补——list 列不管在前(非镜像)
还是在后(镜像)都可能需要跨过分隔线,取决于它是不是 pair 里第一列
(`cols.list_x == 0.0` 就是第一列,不用加;否则要加)。写成条件判断
比死记"哪种情况加"更不容易出错,这里保留显式的 `if mirrored`。**

- [ ] **Step 2: 改写非放大态路径**

找到:

```rust
    let left_w = left_zone_width(window_width, state);
    let (list_w, _) = pair_list_content_width(pair_content_width(left_w), state.dims.files_split);
    let x = byteui::theme::geometry::icon_rail_width() + m.left + p.padding.left;
    let w = (list_w - p.padding.left - p.padding.right).max(0.0);
    let y_pane = byteui::theme::geometry::top_bar_height() + m.top;
    let y = y_pane + p.padding.top + theme::geometry::tree_chrome_top_px();
    let h = ((window_height - m.bottom - byteui::theme::geometry::status_bar_height())
        - (y_pane + p.padding.top + theme::geometry::tree_chrome_top_px())
        - p.padding.bottom
        - theme::geometry::tree_chrome_bottom_px())
    .max(0.0);
    (x, y, w, h)
```

改成:

```rust
    let side = state.layout.rail_layout.side_of(PanelKind::Files);
    let mirrored = side != PanelKind::Files.default_side();
    let (zone_x0, zone_w) = pair_x0_and_width(side, window_width, state);
    let cols = pair_columns(zone_w, state.dims.files_split, mirrored);
    let x = if mirrored {
        zone_x0 + cols.list_x + byteui::theme::geometry::divider_width() + m.left + p.padding.left
    } else {
        zone_x0 + cols.list_x + m.left + p.padding.left
    };
    let w = (cols.list_w - p.padding.left - p.padding.right).max(0.0);
    let y_pane = byteui::theme::geometry::top_bar_height() + m.top;
    let y = y_pane + p.padding.top + theme::geometry::tree_chrome_top_px();
    let h = ((window_height - m.bottom - byteui::theme::geometry::status_bar_height())
        - (y_pane + p.padding.top + theme::geometry::tree_chrome_top_px())
        - p.padding.bottom
        - theme::geometry::tree_chrome_bottom_px())
    .max(0.0);
    (x, y, w, h)
```

**`pair_x0_and_width`(Stage 4a Task 5)返回的第二个值已经是
`pair_content_width(...)` 之后的值(扣过分隔线的配对内容宽)——上面
`pair_columns(zone_w, ...)` 直接用,不要对它再包一层
`pair_content_width`,否则宽度会被多扣一次分隔线,是这个 Task 里最
容易犯的一个量级错误。**

- [ ] **Step 3: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): left_files_tree_bounds 改为 side+镜像感知"
```

---

### Task 6: `is_in_preview_column` 的 `Files`/`Project`/`Web` 分支(非放大态 + 放大态)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- 这个函数只返回 `bool`(命中测试),不需要 `pair_columns` 的完整偏移
  信息,只需要"content 列的起止范围"——可以直接调 `pair_columns` 取
  `content_x`/`content_w` 换算,不需要新辅助函数。

- [ ] **Step 1: 改写放大态里的 `Files`/`Project` 分支**

找到(放大态 `match state.left_view` 块内):

```rust
            PanelKind::Files => {
                let (list_w, _) =
                    pair_list_content_width(pair_content_width(avail_w), state.dims.files_split);
                let start = x0 + list_w + byteui::theme::geometry::divider_width();
                let end = x0 + avail_w;
                x >= start && x < end
            }
```

改成:

```rust
            PanelKind::Files => {
                let mirrored = state.layout.rail_layout.side_of(PanelKind::Files)
                    != PanelKind::Files.default_side();
                let cols =
                    pair_columns(pair_content_width(avail_w), state.dims.files_split, mirrored);
                let start = if mirrored {
                    x0 + cols.content_x
                } else {
                    x0 + cols.content_x + byteui::theme::geometry::divider_width()
                };
                let end = x0 + avail_w;
                x >= start && x < end
            }
```

**`end` 恒是 `x0 + avail_w`(盒子/zone 右边界),不管镜不镜像都不变
——content 列不管渲染在前还是在后,它的右边界都是整个 pair 的右边界
(唯一例外是"content 在前、list 在后"时,content 的右边界其实是
`content_w` 处,不是 `avail_w`——但这个函数原有逻辑本来就是"以content
右边界=zone 右边界"近似处理**,不追求 content 和 list 之间那条分隔线
以内的精确切分,这是延续原有近似,不是这个 Task 引入的新误差。**

`PanelKind::Project` 分支同样手法(`files_split`→`project_split`)。

- [ ] **Step 2: 改写非放大态里的 `Files`/`Project`/`Web` 分支**

找到:

```rust
        PanelKind::Files => {
            let (list_w, _) =
                pair_list_content_width(pair_content_width(left_w), state.dims.files_split);
            let start = byteui::theme::geometry::icon_rail_width()
                + list_w
                + byteui::theme::geometry::divider_width();
            let end = byteui::theme::geometry::icon_rail_width() + left_w;
            x >= start && x < end
        }
```

改成:

```rust
        PanelKind::Files => {
            let side = state.layout.rail_layout.side_of(PanelKind::Files);
            let mirrored = side != PanelKind::Files.default_side();
            let (zone_x0, zone_w) = pair_x0_and_width(side, window_width, state);
            let cols = pair_columns(zone_w, state.dims.files_split, mirrored);
            let start = if mirrored {
                zone_x0 + cols.content_x
            } else {
                zone_x0 + cols.content_x + byteui::theme::geometry::divider_width()
            };
            let end = zone_x0 + zone_w;
            x >= start && x < end
        }
```

`Project` 分支同样手法。`Web` 分支:

找到:

```rust
        // 浏览器(Web)是单栏,占满整条左面板区横向范围。
        PanelKind::Web => {
            let start = byteui::theme::geometry::icon_rail_width();
            let end = start + left_w;
            x >= start && x < end
        }
```

改成:

```rust
        PanelKind::Web => {
            let side = state.layout.rail_layout.side_of(PanelKind::Web);
            let (zone_x0, zone_w) = match side {
                Side::Left => (
                    byteui::theme::geometry::icon_rail_width(),
                    left_zone_width(window_width, state),
                ),
                Side::Right => {
                    let right_w = right_zone_width(window_width, state);
                    (
                        window_width - byteui::theme::geometry::icon_rail_width() - right_w,
                        right_w,
                    )
                }
            };
            x >= zone_x0 && x < zone_x0 + zone_w
        }
```

（`Web` 单栏时(未展开收藏夹)不需要 `pair_columns`,整块 zone 都算
预览列,和改造前逻辑一致,只是 zone 基准现在按 side 选;`bookmarks_
open` 展开时的精确切分,这个函数原有实现本来就没有细分处理——同 Files/
Project 沿用现状的近似程度,不在这个 Task 里额外补精确。）

- [ ] **Step 3: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): is_in_preview_column 的 Files/Project/Web 分支改为 side+镜像感知"
```

---

### Task 7: `apply_column_drag` 的 `LeftPairSplit`/`ProjectSplit`/`BrowserBookmarksSplit`

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

同 Stage 4a Task 5/6 的手法(`pair_x0_and_width`/`list_rendered_first`
两个辅助函数已经在那个 Stage 写好,这里直接复用,不重新定义)。

- [ ] **Step 1: 改写 `Divider::LeftPairSplit`(Files)**

找到:

```rust
        Divider::LeftPairSplit => {
            let pair_w = pair_content_width(left_zone_width(window_width, &state));
            if pair_w <= 0.0 {
                return state.dims;
            }
            let ratio = ((logical_x - byteui::theme::geometry::icon_rail_width()) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            PanelDims {
                files_split: ratio,
                ..state.dims
            }
        }
```

改成(同 Stage 4a `SshSplit` 的改法):

```rust
        Divider::LeftPairSplit => {
            let side = state.layout.rail_layout.side_of(PanelKind::Files);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != PanelKind::Files.default_side();
            let ratio = if list_rendered_first(true, mirrored) {
                raw_ratio
            } else {
                1.0 - raw_ratio
            };
            PanelDims {
                files_split: ratio,
                ..state.dims
            }
        }
```

- [ ] **Step 2: 同样改写 `Divider::ProjectSplit`**(`PanelKind::Files`/
  `files_split` 换成 `PanelKind::Project`/`project_split`)

- [ ] **Step 3: 改写 `Divider::BrowserBookmarksSplit`**

找到:

```rust
        Divider::BrowserBookmarksSplit => {
            let pair_w = pair_content_width(left_zone_width(window_width, &state));
            if pair_w <= 0.0 {
                return state.dims;
            }
            let ratio = ((logical_x - byteui::theme::geometry::icon_rail_width()) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            PanelDims {
                browser_bookmarks_split: ratio,
                ..state.dims
            }
        }
```

改成:

```rust
        Divider::BrowserBookmarksSplit => {
            let side = state.layout.rail_layout.side_of(PanelKind::Web);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != PanelKind::Web.default_side();
            // browser_bookmarks_split 存的是"内容占比"(Web 唯一反着命名
            // 的字段——见 spec 备注),content 默认渲染在前,所以这里传
            // `list_rendered_first` 的 `default_list_first` 参数要传
            // `false`(不是 `true`)——"list" 这个泛化概念在 Web 这里对应
            // 的是收藏夹侧栏,不是内容。
            let ratio = if list_rendered_first(false, mirrored) {
                1.0 - raw_ratio
            } else {
                raw_ratio
            };
            PanelDims {
                browser_bookmarks_split: ratio,
                ..state.dims
            }
        }
```

**这里的翻转方向和 Task 1(Files/Project)刚好相反**——因为
`browser_bookmarks_split` 存的是"content(第一列,`default_list_first
=false` 场景下的'非 list'那侧)占比",不是"list 占比"。`raw_ratio`
恒是"pair 内第一个元素的占比";`list_rendered_first(false, mirrored)`
为真时说明"list(收藏夹)现在渲染在前",那么第一个元素就是收藏夹,
`raw_ratio` 直接是收藏夹占比,要翻转成内容占比(`1.0-raw_ratio`)才是
`browser_bookmarks_split` 该存的值;为假时说明内容渲染在前,`raw_ratio`
本来就是内容占比,直接存。写反了会导致收藏夹拖拽方向錯亂,实现后务必
用 Task 8 的 GUI 核对手动验证一遍这个分支,不要只信编译通过。

- [ ] **Step 4: 追加测试**(同 Stage 4a Task 5/6 的 near/far 方向性
  手法,三个 Divider 各来一组默认栏防回归 + 镜像态方向验证,共 6 条)

- [ ] **Step 5: 编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer apply_column_drag`
Expected: 编译成功,全部通过

- [ ] **Step 6: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): apply_column_drag 的 Files/Project/Web 分支改为 side+镜像感知"
```

---

### Task 8: 全项目收尾全量验证

**Files:**
- 无修改,纯验证

**Interfaces:**
- Consumes: Task 1-7 已完成,以及 Stage 1-4a 全部已合并

- [ ] **Step 1: 全量编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer`
Expected: 编译成功;测试全部通过。

- [ ] **Step 2: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 无新增警告、无格式差异。

- [ ] **Step 3: 全项目 GUI 核对**(这是 4 个 Stage 里唯一一次覆盖全部
  11 个面板、全部能力的收尾验证)

构建一个独立命名的临时二进制,启动后逐条核对:

- **11 个面板全部可以两个方向拖动换栏**,换栏后自动展开在新一侧,
  源栏收起态正确退回相邻面板。
- **8 个有内部两栏布局的面板换栏后内部顺序正确镜像**(Stage 3 已经
  单测验证过逻辑,这里是第一次真正在 GUI 上看到)。
- **`Files`/`Project` 换栏后:webview(文件预览/项目预览)位置与
  镜像后的 iced 布局(项目树/项目信息)对齐**,不叠在树/信息面板上面,
  不悬空,不跑出窗口。文件树的外部拖拽命中(从 Finder 拖文件到树的
  某一行)在换栏后依然准确对应视觉上的行。
- **`Web` 换栏后**:网页 webview 位置正确;收藏夹侧栏展开时同样镜像
  正确、拖拽方向正确。
- **点击焦点路由**(`is_in_preview_column`):换栏后点击预览列(文件
  预览/项目预览/网页),键盘输入正确交给 webview;点击列表列(文件树/
  项目信息/收藏夹),键盘输入不误交给 webview。
- **放大态**(`Ctrl+M` 或既有放大快捷键,核对键位以现有 keymap 为准):
  把已经挪到非默认栏的 `Files`/`Project` 放大,确认放大态下 webview
  位置依然正确镜像(放大盒子本身左右对称,不需要重新验证"盒子在哪",
  只需要验证"盒子内部 list/content 顺序对不对")。
- **8 个非 webview + `Files`/`Project`/`Web` 的内部分割线拖拽,方向在
  镜像态下都正确**(Stage 4a 已经核对过 5 个非 webview 的,这里补
  `Files`/`Project`/`Web` 三个)。
- **不支持栏清空**(Stage 4a 已核对,这里抽查一次确认没有回归)。
- **退出重开**:`RailLayout`、`left_view`/`right_view`、`left_collapsed`/
  `right_collapsed`、`PanelDims` 十个 split 字段全部跨重启保留,数值
  与退出前一致。

完成后关闭该临时实例、删除临时二进制,不留后台进程。

- [ ] **Step 4: Commit(如果 Step 1-3 发现并修复了任何问题)**

```bash
git branch --show-current
git add -A
git commit -m "fix: Stage 4b 全量验证发现的问题修复"
```

(如果全部一次通过,跳过,不产生空 commit。)

---

## 完工验收

1. `git log --oneline` 确认全部 commit 都在当前分支上,没有漂到 `main`。
2. `cargo build`/`cargo test`/`cargo clippy`/`cargo fmt --check` 全绿。
3. Step 3 的全项目 GUI 核对清单逐条通过。
4. 提请审阅。**审阅通过合并后,"workspace 图标栏面板拖拽换栏"这个
   项目(spec + 4 个 Stage)全部完工**——11 个面板均可在左右图标栏间
   自由拖拽,面板跟随打开在新一侧,有内部两栏布局的面板正确镜像渲染
   顺序与分割线拖拽方向,3 个 webview 面板的原生子视图位置正确跟随
   镜像布局,换栏结果跨重启保留。
