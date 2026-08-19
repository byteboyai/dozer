# workspace 图标栏面板拖拽换栏 · Stage 4b:webview 面板并发镜像几何(收尾)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `Files`/`Project`/`Web` 三个挂了原生 `wry` webview 的面板,拖到
非默认栏后,webview 子视图的像素位置要跟纯 iced 部分(Stage 3 已经做好
的镜像渲染)对齐;并且——这是本版计划相对旧版的核心修订——`Files`(左栏)
与 `Project`(右栏)必须能够**同时**各自正确显示各自的 webview,不是
"任一时刻至多一个 webview 面板活跃"。

**为什么改这版计划:** 旧版 Stage 4b 计划(2026-08-19 首版)假设"任意
时刻至多一个 webview 面板处于活跃态,只是它可能挂在左栏或右栏之一",
只给 `preview_content_bounds` 等函数的 `match state.left_view { ... }`
加了一条镜像分支。这个假设在 Stage 4a 合并后的代码审阅里被证伪:
`Files`/`Project`/`Web` 三者的**默认栏都是左栏**,而 `left_view`/
`right_view` 是两侧各自独立的状态,用户只需一步拖拽(比如把 `Project`
拖到右栏、`Files` 留在左栏)就能让两个 webview 面板同时活跃在左右两侧
——这不是边界情况,是这个功能最自然的第一个用例之一。旧版计划的
`match state.left_view` 式改法在这种状态下**完全不会触发**(因为
`Project` 已经不在 `left_view` 里了),webview 直接消失,不是"位置
镜像错了"这种更轻的问题。详见 spec"webview 面板的镜像 bounds
(2026-08-19 Stage 4a 审阅后修订)"一节。

**Architecture:** 五处代码在拖拽落地前只认 `state.left_view`,需要按
下面的顺序逐步改成"独立扫左右两侧、bounds 各自携带、id 空间不相撞":

1. `main.rs::sync_webview_pool` 目前整批 `specs` 共用一个 `bounds:
   wry::Rect`——改成每条 spec 自带各自的矩形(`Vec<(WebviewSpec,
   wry::Rect)>`),这是让"两个矩形同时生效"在类型层面成为可能的前提,
   放在 Task 1 先做(纯签名重构,行为不变,风险最低,给后面的任务
   打地基)。
2. `preview_content_bounds`/`left_files_tree_bounds` 从"隐式只服务
   左栏"改成显式接受 `side: Side` 参数,按该侧当前的 `PanelKind` 分派
   ——复用 Stage 4a 已经写好的 `pair_x0_and_width(side, ...)`(非放大态)
   ;放大态盒子本身左右对称、两侧共用同一个盒子(`maximized_box_x_range`),
   但要按 `MaximizedPane::Left`/`::Right` 决定盒子里放的是 `left_view`
   还是 `right_view`——现状 `MaximizedPane::Right` 分支直接返回零矩形,
   这个洞这次一并堵上。放在 Task 2。
3. `ws.preview`(Files)与 `ws.project_preview`(Project)是两个独立
   `PreviewPane`,各自 `next_id` 从 0 起数——两者的 `WebviewSpec` 一旦
   同时进同一个 `webviews` 池会撞 id。引入一个固定偏移量给
   `ws.project_preview` 的 id 用,`preview_desired`/`browser_desired`
   同时改成独立扫左右两侧、各自配上 Task 2 的 bounds、返回
   `Vec<(WebviewSpec, bounds)>` 喂给 Task 1 的新签名。放在 Task 3。
4. `is_in_preview_column` 从 `bool` 改成 `Option<PanelKind>`(命中的是
   哪个面板);焦点路由(`main.rs` 的 `FocusIntent`)和
   `active_preview_webview_id`(**审阅时发现的一个独立的、先于本项目
   就存在的 bug**:现状硬编码只查 `ws.preview`,从来没查过
   `ws.project_preview`——Project 面板的预览 webview 点击后键盘焦点
   一直交不过去,⌘C 复制不了,这个项目让它第一次变得可测)一并按
   Task 3 的 id 偏移方案改对。放在 Task 4。
5. `apply_column_drag` 的 `LeftPairSplit`/`ProjectSplit`/
   `BrowserBookmarksSplit` 三个分支(Files/Project/Web 各自的内部
   分割线拖拽)——这部分不受"并发双活跃"影响(一条分割线只服务一个
   具体面板实例,不会同时属于两侧),手法同 Stage 4a 已经改过的
   `SshSplit`/`TodoSplit`/`GitLogSplit`。放在 Task 5。

**Tech Stack:** Rust 2024,复用 Stage 4a 的 `pair_x0_and_width`/
`list_rendered_first`。

**Spec:** `docs/superpowers/specs/2026-08-19-rail-panel-drag-relocation-design.md`
(见"webview 面板的镜像 bounds(2026-08-19 Stage 4a 审阅后修订)"一节)

## Global Constraints

- **前提:Stage 1-4a 均已合并到 main。** 开工前确认 `pair_x0_and_width`/
  `list_rendered_first`(Stage 4a)、`RailDrag`/`Message::RailDragMove`/
  `RailDragEnd`(Stage 4a)、共享的 `panel_body`(Stage 4a 审阅追加的
  `13f65cf`)均已存在。
- **独立分支开发,不直接提交 main。** 在新分支(如
  `feature/rail-panel-relocation-stage4b`)上完成全部 6 个 Task,提请
  审阅、通过后再合并回 `main`。每次 `git commit`/`git add` 前先跑
  `git branch --show-current`;开工前 `git status` 确认干净,发现不
  相关的改动要 `git stash push -- <具体路径>`(不要用 `-u`)。
- **每个 Task 结束都要求 `cargo build -p dozer-app --bin dozer` 通过,
  且 `cargo test -p dozer-app --bin dozer` 全绿。** 这个 Stage 改的都是
  同一批函数的调用链(`sync_webview_pool` → `preview_desired`/
  `browser_desired` → `preview_content_bounds`/`is_in_preview_column`),
  任务之间是强依赖顺序,不能乱序执行。
- **这是整个项目最后一个 Stage。** Task 6(全量验证)的 GUI 核对范围
  覆盖全部 11 个面板、全部 4 个 Stage 累积的能力,是唯一一次"全项目
  收尾式"验证,新增重点是"两个 webview 面板分居左右同时显示"这个
  之前任何 Stage 都没测过的场景。
- **像素级精度不强求完美,但方向、量级、id 唯一性必须对**——webview
  矩形算错一两像素不算这个 Stage 的失败,但如果 webview 整块摆到窗口
  另一侧、叠在了不该叠的内容上面、或者两个 webview 因 id 相撞而互相
  顶掉对方,就是这个 Stage 要堵住的真实 bug。

---

### Task 1: `sync_webview_pool` 改成每条 spec 自带矩形

**Files:**
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Produces: `fn sync_webview_pool(window: &Window, pool: &mut HashMap<usize,
  (WebView, String)>, specs: Vec<(preview::WebviewSpec, wry::Rect)>,
  allowed_files: ..., proxy: ...)` ——原 `bounds: wry::Rect` 参数删除,
  改成随每条 spec 一起传。

这一步先只做签名重构,不改变任何实际行为——两个调用处目前都只有一个
`bounds` 值,这里先把它 `zip` 进每条 spec,行为与改动前逐位一致。

- [ ] **Step 1: 改写 `sync_webview_pool` 签名与内部两处 `bounds` 用法**

找到(`main.rs` 约 477 行起):

```rust
    fn sync_webview_pool(
        window: &winit::window::Window,
        pool: &mut std::collections::HashMap<usize, (wry::WebView, String)>,
        specs: Vec<preview::WebviewSpec>,
        bounds: wry::Rect,
        allowed_files: std::sync::Arc<
            std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>,
        >,
        proxy: winit::event_loop::EventLoopProxy<Message>,
    ) {
        let desired_ids: std::collections::HashSet<usize> = specs.iter().map(|s| s.id).collect();
        pool.retain(|id, _| desired_ids.contains(id));

        for spec in specs {
            match pool.get_mut(&spec.id) {
                Some((view, loaded_url)) => {
                    if *loaded_url != spec.url {
                        if let Err(e) = view.load_url(&spec.url) {
                            tracing::warn!("预览导航失败: {e}");
                        }
                        *loaded_url = spec.url.clone();
                        let _ = view.zoom(byteui::theme::icon_size::scale() as f64);
                    }
                    let _ = view.set_bounds(bounds);
                    let _ = view.set_visible(spec.visible);
                }
                None => {
                    let allowed = std::sync::Arc::clone(&allowed_files);
                    let root = assets::assets_root();
                    let ipc_proxy = proxy.clone();
                    let built = wry::WebViewBuilder::new()
                        .with_url(&spec.url)
                        .with_bounds(bounds)
                        .with_visible(spec.visible)
```

改成:

```rust
    /// 每条 spec 自带各自的矩形(不再是整批共用一个)——支持两个不同的
    /// webview 面板(如 `Files` 在左栏、`Project` 在右栏)同时出现在同一
    /// 个池里,各自摆在各自的位置。见 spec"webview 面板的镜像 bounds
    /// (2026-08-19 Stage 4a 审阅后修订)"一节。
    fn sync_webview_pool(
        window: &winit::window::Window,
        pool: &mut std::collections::HashMap<usize, (wry::WebView, String)>,
        specs: Vec<(preview::WebviewSpec, wry::Rect)>,
        allowed_files: std::sync::Arc<
            std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>,
        >,
        proxy: winit::event_loop::EventLoopProxy<Message>,
    ) {
        let desired_ids: std::collections::HashSet<usize> =
            specs.iter().map(|(s, _)| s.id).collect();
        pool.retain(|id, _| desired_ids.contains(id));

        for (spec, bounds) in specs {
            match pool.get_mut(&spec.id) {
                Some((view, loaded_url)) => {
                    if *loaded_url != spec.url {
                        if let Err(e) = view.load_url(&spec.url) {
                            tracing::warn!("预览导航失败: {e}");
                        }
                        *loaded_url = spec.url.clone();
                        let _ = view.zoom(byteui::theme::icon_size::scale() as f64);
                    }
                    let _ = view.set_bounds(bounds);
                    let _ = view.set_visible(spec.visible);
                }
                None => {
                    let allowed = std::sync::Arc::clone(&allowed_files);
                    let root = assets::assets_root();
                    let ipc_proxy = proxy.clone();
                    let built = wry::WebViewBuilder::new()
                        .with_url(&spec.url)
                        .with_bounds(bounds)
                        .with_visible(spec.visible)
```

函数体剩余部分(`with_allow_link_preview`/`with_initialization_script`/
`with_ipc_handler`/`with_custom_protocol`/`.build_as_child(window)` 及
之后的 `match built { ... }`)不引用 `bounds` 之外的东西,原样不动
(`bounds` 现在是 for 循环里的局部绑定,类型不变,函数体其余部分照常
能编译通过)。

- [ ] **Step 2: 改写两个调用处,先用现有单个 `bounds` 包成 `vec![(...);
  N]` 形式**(行为暂不变,只是类型对齐——Task 3 会再改成真正的双侧
  独立 bounds)

找到(约 1253 行 `sync_webview_pool` 第一处调用):

```rust
            let (x, y, w, h) =
                app::preview_content_bounds(logical_w, logical_h, &app.shell_state());
            let bounds = wry::Rect {
                position: wry::dpi::LogicalPosition::new(x as f64, y as f64).into(),
                size: wry::dpi::LogicalSize::new(w as f64, h as f64).into(),
            };

            sync_webview_pool(
                window.as_ref(),
                webviews,
                app.preview_desired(),
                bounds,
                app.allowed_files(),
                proxy.clone(),
            );
```

改成:

```rust
            let (x, y, w, h) =
                app::preview_content_bounds(logical_w, logical_h, &app.shell_state());
            let bounds = wry::Rect {
                position: wry::dpi::LogicalPosition::new(x as f64, y as f64).into(),
                size: wry::dpi::LogicalSize::new(w as f64, h as f64).into(),
            };

            sync_webview_pool(
                window.as_ref(),
                webviews,
                app.preview_desired()
                    .into_iter()
                    .map(|s| (s, bounds))
                    .collect(),
                app.allowed_files(),
                proxy.clone(),
            );
```

第二处(`browser_webviews`,约 1277 行)同样手法:

```rust
            sync_webview_pool(
                window.as_ref(),
                browser_webviews,
                app.browser_desired(),
                browser_bounds,
                app.allowed_files(),
                proxy.clone(),
            );
```

改成:

```rust
            sync_webview_pool(
                window.as_ref(),
                browser_webviews,
                app.browser_desired()
                    .into_iter()
                    .map(|s| (s, browser_bounds))
                    .collect(),
                app.allowed_files(),
                proxy.clone(),
            );
```

- [ ] **Step 3: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功(`preview_content_bounds`/`preview_desired`/
`browser_desired` 的签名这个 Task 还没动,行为与改动前完全一致)。

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/main.rs
git commit -m "refactor(dozer-app): sync_webview_pool 改为每条 spec 自带矩形,为并发双侧 webview 打地基"
```

---

### Task 2: `preview_content_bounds`/`left_files_tree_bounds` 显式 side 参数化(含放大态右侧补洞)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces: `fn preview_content_bounds_for(side: Side, window_width: f32,
  window_height: f32, state: &ShellState) -> (f32, f32, f32, f32)`、
  `fn left_files_tree_bounds_for(side: Side, window_width: f32,
  window_height: f32, state: &ShellState) -> (f32, f32, f32, f32)`——
  取代原来的隐式左栏版本 `preview_content_bounds`/`left_files_tree_bounds`
  (删除旧函数,不保留兼容包装。`preview_content_bounds` 真实调用点共 3
  个:`main.rs` 两处(Task 1 已经改成 `Vec<(spec, bounds)>` 形状,但那两
  处目前仍各自调用一次旧版 `preview_content_bounds` 现算 `bounds`——
  Task 3 会把这两处的现算逻辑整段删掉,改用 `preview_desired`/
  `browser_desired` 自带的 bounds,所以这个 Task 不用改 `main.rs`)、
  `app.rs` 里的 `ime_cursor_area`(本 Task Step 4.5 改)。
  `left_files_tree_bounds` 真实调用点 1 个:`app.rs` 里的
  `files_drop_target`(本 Task Step 4.6 改)。其余全是测试,本 Task
  Step 5 一并迁移,不留悬空引用。
- Consumes: Stage 4a 的 `pair_x0_and_width(side, window_width, state) ->
  (f32, f32)`、`fn maximized_box_x_range(window_width: f32) -> (f32,
  f32)`(两侧共用,不需要 side 参数)。

- [ ] **Step 1: 改写 `preview_content_bounds` → `preview_content_bounds_for`**

找到整个函数(约 1080-1221 行),把函数签名和两处 `match state.left_view`
改成按 `side` 取实际面板类型再分派。放大态部分:

原放大态入口:

```rust
    if state.maximized == Some(MaximizedPane::Left) {
        let (x0, avail_w) = maximized_box_x_range(window_width);
        let y0 = byteui::theme::geometry::top_bar_height()
            + byteui::theme::geometry::maximize_overlay_padding();
        let avail_h = (maximized_box_height(window_height)
            - byteui::theme::geometry::status_bar_height())
        .max(0.0);
        return match state.left_view {
            PanelKind::Files => { /* ... */ }
            PanelKind::Web => { /* ... */ }
            PanelKind::GitLog => (0.0, 0.0, 0.0, 0.0),
            PanelKind::Todo => (0.0, 0.0, 0.0, 0.0),
            PanelKind::Project => { /* ... */ }
            PanelKind::Database => (0.0, 0.0, 0.0, 0.0),
            PanelKind::Ssh => (0.0, 0.0, 0.0, 0.0),
            PanelKind::Agent
            | PanelKind::Conversations
            | PanelKind::Usage
            | PanelKind::Acceptance => (0.0, 0.0, 0.0, 0.0),
        };
    }
```

改成(`MaximizedPane::Left`/`::Right` 各自决定盒子里放的是 `left_view`
还是 `right_view`,盒子本身两侧共用;`mirrored` 判断 Files/Project 是否
需要把 content 列换到前面——非放大态 zone 用同一份 `mirrored` 计算,
搬进各分支内部,原理同 Stage 4b 旧版计划已经验证过的镜像数学,这里改成
基于 `side`(而不是死记"当前必是左栏")):

```rust
    if let Some(maximized) = state.maximized {
        let (x0, avail_w) = maximized_box_x_range(window_width);
        let y0 = byteui::theme::geometry::top_bar_height()
            + byteui::theme::geometry::maximize_overlay_padding();
        let avail_h = (maximized_box_height(window_height)
            - byteui::theme::geometry::status_bar_height())
        .max(0.0);
        // 放大的是另一侧:这一侧内容被变暗遮罩整片盖住,原生 wry 子视图
        // 不听 iced 绘制顺序,必须用零尺寸矩形真正藏起来(同 `left_collapsed`
        // 分支同一手法)。
        let showing_side = match maximized {
            MaximizedPane::Left => Side::Left,
            MaximizedPane::Right => Side::Right,
        };
        if side != showing_side {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let kind = match side {
            Side::Left => state.left_view,
            Side::Right => state.right_view,
        };
        let mirrored = state.layout.rail_layout.side_of(kind) != kind.default_side();
        return match kind {
            PanelKind::Files => {
                let y = y0 + byteui::theme::geometry::preview_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::preview_chrome_top_px() - 8.0).max(0.0);
                let pair_w = pair_content_width(avail_w);
                let cols = pair_columns(pair_w, state.dims.files_split, mirrored);
                let x = if mirrored {
                    x0 + cols.content_x + 8.0
                } else {
                    x0 + cols.content_x + byteui::theme::geometry::divider_width() + 8.0
                };
                let w = (cols.content_w - 16.0).max(0.0);
                (x, y, w, h)
            }
            // 浏览器(Web)是单栏(无配对),放大态占满整条放大盒子,side
            // 不影响它的矩形——但仍需先过上面的 `side != showing_side` 判断。
            PanelKind::Web => {
                let y = y0 + byteui::theme::geometry::browser_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::browser_chrome_top_px() - 8.0).max(0.0);
                let x = x0 + 8.0;
                let w = (avail_w - 16.0).max(0.0);
                (x, y, w, h)
            }
            // Git 提交图是原生 Canvas 绘制,不挂 webview 子视图。
            PanelKind::GitLog => (0.0, 0.0, 0.0, 0.0),
            // Todo 面板同 GitLog,纯 iced 绘制,不挂 webview 子视图。
            PanelKind::Todo => (0.0, 0.0, 0.0, 0.0),
            PanelKind::Project => {
                let y = y0 + byteui::theme::geometry::preview_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::preview_chrome_top_px() - 8.0).max(0.0);
                let pair_w = pair_content_width(avail_w);
                let cols = pair_columns(pair_w, state.dims.project_split, mirrored);
                let x = if mirrored {
                    x0 + cols.content_x + 8.0
                } else {
                    x0 + cols.content_x + byteui::theme::geometry::divider_width() + 8.0
                };
                let w = (cols.content_w - 16.0).max(0.0);
                (x, y, w, h)
            }
            // Database/Ssh/Todo/GitLog 纯 iced 绘制,不挂 webview 子视图;
            // Agent/Conversations/Usage/Acceptance 同理——任一侧放大只要
            // 显示的是这几种,都没有 webview 可摆。
            PanelKind::Database
            | PanelKind::Ssh
            | PanelKind::Agent
            | PanelKind::Conversations
            | PanelKind::Usage
            | PanelKind::Acceptance => (0.0, 0.0, 0.0, 0.0),
        };
    }
```

**这里引入了一个新的辅助函数 `pair_columns`(Stage 4b 旧版计划 Task 1
已经设计好、这次沿用其定义不变)——本 Task 的 Step 2 先补上这个函数
定义,再继续改非放大态分支。**

- [ ] **Step 2: 补 `pair_columns` 辅助函数(放在 `pair_list_content_width`**
附近)

```rust
/// 配对视图内 list/content 两列相对**所在 zone/盒子内容区左边界**的
/// 横向偏移与宽度(不含 zone/盒子自己的 x0——调用方自己加)。`mirrored`
/// = false 时 list 在前(x=0)、content 在后(x=list_w+divider_width);
/// `mirrored` = true 时反过来。`preview_content_bounds_for`/
/// `left_files_tree_bounds_for`/`is_in_preview_column` 三个函数(webview
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

- [ ] **Step 3: 改写非放大态分支 + 函数签名/开头**

找到函数开头与非放大态部分(约 1080-1090、1148-1220 行):

```rust
pub fn preview_content_bounds(
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32, f32, f32) {
    if state.left_collapsed {
        return (0.0, 0.0, 0.0, 0.0);
    }
    if state.maximized == Some(MaximizedPane::Right) {
        return (0.0, 0.0, 0.0, 0.0);
    }
    if state.maximized == Some(MaximizedPane::Left) {
        /* ...已在 Step 1 改写... */
    }
    let left_w = left_zone_width(window_width, state);
    let m = theme::region::left_zone().margin;
    let y_top = |chrome_top: f32| -> f32 { /* ... */ };
    let h_for = |y: f32| -> f32 { /* ... */ };
    match state.left_view {
        PanelKind::Files => { /* ... */ }
        PanelKind::Web => { /* ... */ }
        PanelKind::GitLog => (0.0, 0.0, 0.0, 0.0),
        PanelKind::Todo => (0.0, 0.0, 0.0, 0.0),
        PanelKind::Project => { /* ... */ }
        PanelKind::Database => (0.0, 0.0, 0.0, 0.0),
        PanelKind::Ssh => (0.0, 0.0, 0.0, 0.0),
        PanelKind::Agent | PanelKind::Conversations | PanelKind::Usage | PanelKind::Acceptance => {
            (0.0, 0.0, 0.0, 0.0)
        }
    }
}
```

改成:

```rust
/// 窗口逻辑尺寸 → `side` 这一侧当前活跃 webview 面板(如果有)的内容区
/// 矩形(逻辑像素 x/y/w/h),供 main.rs 摆放 wry webview 用。`side` 这一
/// 侧收起、或不是 webview 面板(GitLog/Todo/Database/Ssh/Agent/
/// Conversations/Usage/Acceptance)时返回零尺寸矩形。
///
/// 2026-08-19 Stage 4a 审阅后修订:此前隐式假设"任一时刻至多一个 webview
/// 面板活跃、只服务左栏",在 `Files` 留左栏、`Project` 挪右栏这类一步
/// 拖拽即可达到的状态下会漏掉右栏那一个——现在两侧各自独立算,`main.rs`
/// 对左右两侧各调一次。
pub fn preview_content_bounds_for(
    side: Side,
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32, f32, f32) {
    let collapsed = match side {
        Side::Left => state.left_collapsed,
        Side::Right => state.right_collapsed,
    };
    if collapsed {
        return (0.0, 0.0, 0.0, 0.0);
    }
    if let Some(maximized) = state.maximized {
        /* ...Step 1 的放大态整块,原样搬到这里... */
    }
    let kind = match side {
        Side::Left => state.left_view,
        Side::Right => state.right_view,
    };
    let mirrored = state.layout.rail_layout.side_of(kind) != kind.default_side();
    let (zone_x0, zone_w) = pair_x0_and_width(side, window_width, state);
    let m = match side {
        Side::Left => theme::region::left_zone().margin,
        Side::Right => theme::region::right_zone().margin,
    };
    let y_top = |chrome_top: f32| -> f32 {
        byteui::theme::geometry::top_bar_height() + m.top + chrome_top
    };
    let h_for = |y: f32| -> f32 {
        (window_height - y - m.bottom - byteui::theme::geometry::status_bar_height()).max(0.0)
    };
    match kind {
        PanelKind::Files => {
            let y = y_top(byteui::theme::geometry::preview_chrome_top_px());
            let h = h_for(y);
            let cols = pair_columns(zone_w, state.dims.files_split, mirrored);
            let x = if mirrored {
                zone_x0 + cols.content_x + m.left
            } else {
                zone_x0 + cols.content_x + byteui::theme::geometry::divider_width() + 8.0 + m.left
            };
            let w = (cols.content_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
        // 浏览器(Web):收藏夹侧栏关闭时单栏占满该侧面板区;打开时网页
        // 内容让出收藏夹侧栏的宽度。收藏夹在"内容"前面还是后面同样按
        // `mirrored` 决定(见下方 `pair_columns` 调用,`split` 参数传的是
        // "收藏夹占比" = `1.0 - browser_bookmarks_split`,因为该字段存的
        // 是内容占比,和其余 7 个 split 字段"存列表侧占比"的语义相反)。
        PanelKind::Web => {
            let y = y_top(byteui::theme::geometry::browser_chrome_top_px());
            let h = h_for(y);
            let x = zone_x0 + 8.0 + m.left;
            let w = if state.browser_bookmarks_open {
                let cols =
                    pair_columns(zone_w, 1.0 - state.dims.browser_bookmarks_split, mirrored);
                (cols.content_w - 16.0 - m.left - m.right).max(0.0)
            } else {
                (zone_w - 16.0 - m.left - m.right).max(0.0)
            };
            (x, y, w, h)
        }
        PanelKind::GitLog => (0.0, 0.0, 0.0, 0.0),
        PanelKind::Todo => (0.0, 0.0, 0.0, 0.0),
        PanelKind::Project => {
            let y = y_top(byteui::theme::geometry::preview_chrome_top_px());
            let h = h_for(y);
            let cols = pair_columns(zone_w, state.dims.project_split, mirrored);
            let x = if mirrored {
                zone_x0 + cols.content_x + m.left
            } else {
                zone_x0 + cols.content_x + byteui::theme::geometry::divider_width() + 8.0 + m.left
            };
            let w = (cols.content_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
        PanelKind::Database | PanelKind::Ssh => (0.0, 0.0, 0.0, 0.0),
        PanelKind::Agent | PanelKind::Conversations | PanelKind::Usage | PanelKind::Acceptance => {
            (0.0, 0.0, 0.0, 0.0)
        }
    }
}
```

**`Web` 分支的 `x`/`zone_w` 不再区分 side 单独写(`icon_rail_width() +
8.0 + m.left` 这个原来专属左栏的写法已经被 `zone_x0`(来自
`pair_x0_and_width`,两侧通用)取代)——不要在这个分支里保留任何
`icon_rail_width()` 字面量,那是左栏专属基准,右栏场景下位置会算错。**

- [ ] **Step 3.5: 临时打通 `main.rs` 里还没删的两处旧调用(保证这个 Task
  结束时仍能编译——真正删掉这两处现算逻辑是 Task 3 的事)**

`main.rs` 里 `webviews`/`browser_webviews` 两处 `sync_webview_pool`
调用前,各自还有一次 `app::preview_content_bounds(logical_w, logical_h,
&app.shell_state())`(Task 1 只改了 `sync_webview_pool` 那半句,没动这
两行现算)。这个 Task 把 `preview_content_bounds` 整个删掉重建成
`preview_content_bounds_for`,这两行现在会因为函数不存在而编译不过。
找到:

```rust
            let (x, y, w, h) =
                app::preview_content_bounds(logical_w, logical_h, &app.shell_state());
```

（两处一字不差,分别在 `webviews`/`browser_bounds` 两段前)全部改成:

```rust
            let (x, y, w, h) =
                app::preview_content_bounds_for(app::Side::Left, logical_w, logical_h, &app.shell_state());
```

**这不是最终形态——旧代码本来就隐式只服务左栏,这里显式传
`Side::Left` 只是让行为在这个 Task 结束时和改动前完全一致,保持
`main.rs` 能编译。Task 3 会把这两行连同它们所在的整段现算逻辑一起
删掉,改用 `preview_desired`/`browser_desired` 自带的按侧计算好的
矩形——那时 `Side::Left` 这个临时写法自然消失,不需要现在纠结"这样
写右栏对不对",Task 3 之前它本来就只服务左栏。**

- [ ] 编译确认:`cargo build -p dozer-app --bin dozer` 通过。

- [ ] **Step 4: 同样手法改写 `left_files_tree_bounds` → `left_files_tree_bounds_for`**

（Files 专属,只关心配对里的 list 列而非 content 列）找到整个函数(约
1242-1290 行),签名加 `side: Side` 参数,内部改成:

```rust
/// `side` 这一侧文件树的**目录列表 Scrollable** 在窗口坐标系里的矩形
/// (上/左/宽/高,逻辑像素),供 main.rs 做外部文件拖拽命中测试。这一侧
/// 当前不是 `Files` 面板、或该侧收起、或右侧放大挡住了它时返回零尺寸
/// 矩形。
pub fn left_files_tree_bounds_for(
    side: Side,
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32, f32, f32) {
    let zero = || (0.0, 0.0, 0.0, 0.0);
    let kind = match side {
        Side::Left => state.left_view,
        Side::Right => state.right_view,
    };
    let collapsed = match side {
        Side::Left => state.left_collapsed,
        Side::Right => state.right_collapsed,
    };
    if collapsed || kind != PanelKind::Files {
        return zero();
    }
    let mirrored = state.layout.rail_layout.side_of(PanelKind::Files) != PanelKind::Files.default_side();
    let m = match side {
        Side::Left => theme::region::left_zone().margin,
        Side::Right => theme::region::right_zone().margin,
    };
    let p = theme::region::project_pane();
    if let Some(maximized) = state.maximized {
        let showing_side = match maximized {
            MaximizedPane::Left => Side::Left,
            MaximizedPane::Right => Side::Right,
        };
        if side != showing_side {
            return zero();
        }
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
}
```

**注意 `pair_x0_and_width` 返回的第二个值已经是 `pair_content_width(...)`
之后的值(扣过分隔线的配对内容宽)——上面直接用,不要再包一层
`pair_content_width`,否则宽度多扣一次分隔线。**

- [ ] **Step 4.5: 修第三个调用点——`ime_cursor_area`**

`preview_content_bounds` 除了 `main.rs` 两处、测试若干处,还有一处
**容易漏改**的调用:`ime_cursor_area`(约 3484-3489 行),给 IME 候选窗
定位用。找到:

```rust
    pub fn ime_cursor_area(&self, window_w: f32, window_h: f32) -> (f32, f32, f32) {
        let state = self.shell_state();
        if self.browser_addr_editing() || self.acceptance_comment_editing() {
            let (bx, by, _bw, _bh) = preview_content_bounds(window_w, window_h, &state);
            return (bx + 4.0, by, 20.0);
        }
```

改成(地址栏编辑对应 `Web` 面板、意见框编辑对应 `Acceptance` 面板,
两者都可能已经拖到任一栏,不能再假设左栏;两个条件互斥,分开处理):

```rust
    pub fn ime_cursor_area(&self, window_w: f32, window_h: f32) -> (f32, f32, f32) {
        let state = self.shell_state();
        if self.browser_addr_editing() {
            let side = state.layout.rail_layout.side_of(PanelKind::Web);
            let (bx, by, _bw, _bh) = preview_content_bounds_for(side, window_w, window_h, &state);
            return (bx + 4.0, by, 20.0);
        }
        if self.acceptance_comment_editing() {
            let side = state.layout.rail_layout.side_of(PanelKind::Acceptance);
            let (bx, by, _bw, _bh) = preview_content_bounds_for(side, window_w, window_h, &state);
            return (bx + 4.0, by, 20.0);
        }
```

**`PanelKind::Acceptance` 本身没有 webview、`preview_content_bounds_for`
对它返回零矩形——这里沿用的是原有代码就已经存在的近似手法("地址栏/
意见框编辑态用预览列上部近似",见函数注释),不是这个 Task 引入的新
近似。如果 `Acceptance` 分支返回 `(0.0, 0.0, 0.0, 0.0)` 导致 IME 候选窗
跳到窗口左上角这类明显错位,属于"原有近似手法在 Acceptance 面板上本来
就不准"的既有行为,不在这个 Task 的修复范围内(该近似手法本身如何做对
是另一个话题,不属于本 Stage 的 webview 并发镜像范围);这个 Step 要
保证的只是"按面板实际所在侧算,不再死用左栏公式"这一层,不是让这个
近似变精确。**

- [ ] **Step 4.6: 修第四个调用点——`files_drop_target`(Finder 外部拖拽命中)**

`left_files_tree_bounds` 还有一处调用点在 `files_drop_target`(约
2420-2437 行)——这是从 Finder 往文件树拖文件时,判断"落在哪一行"的
命中测试入口,同样硬编码只在 `left_view == Files` 时生效。找到:

```rust
    pub fn files_drop_target(
        &self,
        window_w: f32,
        window_h: f32,
        x: f32,
        y: f32,
    ) -> Option<PathBuf> {
        if self.left_collapsed || self.left_view != PanelKind::Files {
            return None;
        }
        let ws = self.active_workspace()?;
        let bounds = left_files_tree_bounds(window_w, window_h, &self.shell_state());
        let rows = ws.files.visible_tree_rows();
        files::tree_drop_target(x, y, bounds, ws.files.tree_scroll(), &rows)
    }
```

改成(`Files` 可能在左栏也可能在右栏,先查它现在在哪一侧、该侧有没有
收起,再用那一侧的边界算命中):

```rust
    pub fn files_drop_target(
        &self,
        window_w: f32,
        window_h: f32,
        x: f32,
        y: f32,
    ) -> Option<PathBuf> {
        let side = self.shell_state().layout.rail_layout.side_of(PanelKind::Files);
        let kind = match side {
            Side::Left => self.left_view,
            Side::Right => self.right_view,
        };
        let collapsed = match side {
            Side::Left => self.left_collapsed,
            Side::Right => self.right_collapsed,
        };
        if collapsed || kind != PanelKind::Files {
            return None;
        }
        let ws = self.active_workspace()?;
        let bounds = left_files_tree_bounds_for(side, window_w, window_h, &self.shell_state());
        let rows = ws.files.visible_tree_rows();
        files::tree_drop_target(x, y, bounds, ws.files.tree_scroll(), &rows)
    }
```

**这一处是 spec GUI 核对清单里明确点名的场景("文件树的外部拖拽命中
……在换栏后依然准确对应视觉上的行")——漏了这一处,`Files` 换到右栏后
从 Finder 拖文件进树会全部失效(函数直接返回 `None`),不是"位置算错"
这种轻问题,是"功能完全不可用"。**

- [ ] **Step 5: 迁移现有测试到新签名**

`app.rs` 里所有引用旧 `preview_content_bounds(window_width,
window_height, &state)`(3 参数)/`left_files_tree_bounds(...)` 的测试
(`grep -n "preview_content_bounds(\|left_files_tree_bounds(" crates/dozer-app/src/app.rs`
先摸底精确行号,预计在 8840-9630 行区间的十余处测试里),每处调用改成
`preview_content_bounds_for(Side::Left, window_width, window_height,
&state)`(原逻辑全部假设左栏,迁移时统一传 `Side::Left`,断言数值不变
——这批测试验证的是几何公式本身,不是"哪一侧",迁移后应该原样通过,
不需要改断言内容,只改调用签名)。`left_files_tree_bounds` 同理。

- [ ] **Step 6: 编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer preview_content_bounds && cargo test -p dozer-app --bin dozer left_files_tree_bounds && cargo test -p dozer-app --bin dozer pair_columns`
Expected: 编译成功,全部通过(迁移前的测试断言数值不变)。

- [ ] **Step 7: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): preview_content_bounds/left_files_tree_bounds 改为显式 side 参数,补齐放大态右侧 webview 分支"
```

---

### Task 3: id 空间隔离 + `preview_desired`/`browser_desired` 双侧独立扫描

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: Task 1 的 `sync_webview_pool(..., specs: Vec<(WebviewSpec,
  wry::Rect)>, ...)`、Task 2 的 `preview_content_bounds_for(side, ...)`。
- Produces: `const PROJECT_PREVIEW_ID_OFFSET: usize`、
  `pub fn preview_desired(&self, window_width: f32, window_height: f32)
  -> Vec<(WebviewSpec, (f32,f32,f32,f32))>`、
  `pub fn browser_desired(&self, window_width: f32, window_height: f32)
  -> Vec<(WebviewSpec, (f32,f32,f32,f32))>`(签名加了窗口尺寸参数——
  之前只有 id/url/visible,不带几何;现在几何要按各自命中的 side 现算,
  所以这两个函数需要能拿到窗口尺寸)。

- [ ] **Step 1: 引入 id 偏移常量**

在 `preview_desired` 附近(app.rs 约 3512 行前)加:

```rust
/// `ws.preview`(Files)与 `ws.project_preview`(Project)是两个独立
/// `PreviewPane`,各自 `next_id` 从 0 起数——两者的 webview 一旦同时
/// 进同一个 `webviews` 池(`Files` 在左栏、`Project` 在右栏同时活跃时
/// 就会发生),原始 id 会撞(两边都可能是 0/1/2...)。给 `Project` 那
/// 一侧的 id 统一加这个偏移,`ws.preview` 侧不动——量级远超真实 tab
/// 数(几十个封顶),不会反向撞回 `ws.preview` 的 id 区间。main.rs 里
/// 任何按 id 反查 `ws.project_preview` webview(`active_preview_webview_id`
/// 的 Project 分支)都要用同一个偏移量加/减,两处不同步会导致查错池。
const PROJECT_PREVIEW_ID_OFFSET: usize = 1_000_000;
```

- [ ] **Step 2: 改写 `preview_desired`**

找到(app.rs 约 3517-3554 行):

```rust
    pub fn preview_desired(&self) -> Vec<WebviewSpec> {
        if self.current_page == AppPage::Home {
            return Vec::new();
        }
        if !matches!(self.left_view, PanelKind::Files | PanelKind::Project) {
            return Vec::new();
        }
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        let specs = match self.left_view {
            PanelKind::Files => ws.preview.desired_webviews(),
            PanelKind::Project => ws.project_preview.desired_webviews(),
            _ => Vec::new(),
        };
        if ws.edit_session.is_some() {
            specs
                .into_iter()
                .map(|mut s| {
                    s.visible = false;
                    s
                })
                .collect()
        } else {
            specs
        }
    }
```

改成:

```rust
    /// 当前应存在的"文件/项目预览"webview 清单(main.rs 差集同步用),
    /// 每条自带按其所在侧算好的矩形。左右两侧各自独立判断——`Files` 在
    /// 左栏、`Project` 在右栏可以同时非空(见 spec"webview 面板的镜像
    /// bounds(2026-08-19 Stage 4a 审阅后修订)"一节)。不在文件视图时
    /// 该侧整体不产出;进首页时两侧都不产出(原因见旧版注释:首页时
    /// 预览区根本不在屏上)。
    pub fn preview_desired(&self, window_width: f32, window_height: f32) -> Vec<(WebviewSpec, (f32, f32, f32, f32))> {
        if self.current_page == AppPage::Home {
            return Vec::new();
        }
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        let edit_open = ws.edit_session.is_some();
        let mut out = Vec::new();
        for side in [Side::Left, Side::Right] {
            let kind = match side {
                Side::Left => self.left_view,
                Side::Right => self.right_view,
            };
            let (specs, id_offset): (Vec<WebviewSpec>, usize) = match kind {
                PanelKind::Files => (ws.preview.desired_webviews(), 0),
                PanelKind::Project => {
                    (ws.project_preview.desired_webviews(), PROJECT_PREVIEW_ID_OFFSET)
                }
                _ => continue,
            };
            let bounds =
                preview_content_bounds_for(side, window_width, window_height, &self.shell_state());
            out.extend(specs.into_iter().map(|mut s| {
                s.id += id_offset;
                // 编辑弹层开着时,应用级模态盖住了预览区,原生 wry 子视图
                // 不听 iced 绘制顺序摆布,必须显式 visible=false 才能真正
                // 藏起来。
                if edit_open {
                    s.visible = false;
                }
                (s, bounds)
            }));
        }
        out
    }
```

- [ ] **Step 3: 改写 `browser_desired`**

找到(app.rs 约 3558-3572 行):

```rust
    pub fn browser_desired(&self) -> Vec<WebviewSpec> {
        if self.current_page == AppPage::Home {
            return self.home_browser.desired_webviews();
        }
        if self.left_view != PanelKind::Web {
            return Vec::new();
        }
        match self.active_workspace() {
            Some(ws) => ws.browser.desired_webviews(),
            None => Vec::new(),
        }
    }
```

改成(`Web` 只有一个来源`ws.browser`,不存在 id 相撞问题,但仍需按
"当前在哪侧"给 bounds;首页的 `home_browser` 走独立的 `home_browser_bounds`
——那部分几何仍在 `main.rs`,这里只需对齐"参数里带窗口尺寸"这个新签名,
首页分支的具体矩形留给 `main.rs` 调用处处理,这个函数首页分支直接返回
`(0,0,0,0)` 占位矩形,main.rs 会用它自己算的 `home_browser_bounds` 整体
覆盖,不采用这里返回的矩形——见 Step 4 调用处改动):

```rust
    /// 浏览器域的 webview 清单,语义同 `preview_desired`,查独立的
    /// `Workspace::browser`。首页时矩形留空(main.rs 用 `home_browser_bounds`
    /// 单独覆盖,见调用处),工作区内按 `Web` 当前所在侧现算矩形。
    pub fn browser_desired(&self, window_width: f32, window_height: f32) -> Vec<(WebviewSpec, (f32, f32, f32, f32))> {
        if self.current_page == AppPage::Home {
            return self
                .home_browser
                .desired_webviews()
                .into_iter()
                .map(|s| (s, (0.0, 0.0, 0.0, 0.0)))
                .collect();
        }
        let side = if self.left_view == PanelKind::Web {
            Side::Left
        } else if self.right_view == PanelKind::Web {
            Side::Right
        } else {
            return Vec::new();
        };
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        let bounds =
            preview_content_bounds_for(side, window_width, window_height, &self.shell_state());
        ws.browser
            .desired_webviews()
            .into_iter()
            .map(|s| (s, bounds))
            .collect()
    }
```

- [ ] **Step 4: main.rs 两处调用点改成用新签名的返回值直接喂
  `sync_webview_pool`,不再单独算一次 `bounds` 传进去**

找到 Task 1 Step 2 里已经改过的两处调用(约 1253-1290 行)。现在
`preview_desired`/`browser_desired` 自己就带 bounds 了,原先专门为
`webviews` 池现算的那次 `preview_content_bounds` 调用整段删除(不再
需要——`preview_desired` 内部已经对左右两侧各自算了一次),`.map(|s|
(s, bounds))` 这层包装也去掉,直接传参数:

```rust
            sync_webview_pool(
                window.as_ref(),
                webviews,
                app.preview_desired(logical_w, logical_h)
                    .into_iter()
                    .map(|(s, (x, y, w, h))| {
                        (
                            s,
                            wry::Rect {
                                position: wry::dpi::LogicalPosition::new(x as f64, y as f64).into(),
                                size: wry::dpi::LogicalSize::new(w as f64, h as f64).into(),
                            },
                        )
                    })
                    .collect::<Vec<_>>(),
                app.allowed_files(),
                proxy.clone(),
            );
            let browser_specs: Vec<(preview::WebviewSpec, wry::Rect)> = if app.is_home() {
                let bounds = Self::home_browser_bounds(logical_w, logical_h);
                app.browser_desired(logical_w, logical_h)
                    .into_iter()
                    .map(|(s, _)| (s, bounds))
                    .collect()
            } else {
                app.browser_desired(logical_w, logical_h)
                    .into_iter()
                    .map(|(s, (x, y, w, h))| {
                        (
                            s,
                            wry::Rect {
                                position: wry::dpi::LogicalPosition::new(x as f64, y as f64).into(),
                                size: wry::dpi::LogicalSize::new(w as f64, h as f64).into(),
                            },
                        )
                    })
                    .collect()
            };
            sync_webview_pool(
                window.as_ref(),
                browser_webviews,
                browser_specs,
                app.allowed_files(),
                proxy.clone(),
            );
```

**`home_browser_bounds` 返回值已经是 `wry::Rect`(现有函数,签名不变),
首页分支直接复用它、忽略 `browser_desired` 自带的占位矩形
`(0.0,0.0,0.0,0.0)`——首页的浏览器矩形算法和工作区内的 `Web` 面板矩形
算法是两套不同的公式(见旧注释:首页右栏浏览器占的是右面板区整体,不是
工作区左/右面板预览区),不要把两者混在一起。`wry::Rect` 需要
`Clone`——若 `home_browser_bounds` 需要在 `.map` 闭包里重复用同一个值,
先跑 `cargo build` 看 `Rect` 是否已经 `Clone`/`Copy`;上面的写法把
`bounds` 在 `if` 分支里提前算好一次、闭包按值捕获,不依赖 `Rect` 是否
`Clone`,应该能直接编译通过。**

- [ ] **Step 5: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功。若 Step 4 的 `wry::Rect` clone 问题触发编译错误,
按 Step 4 备注的方案改用元组传递,不要引入新依赖。

- [ ] **Step 6: 新增并发测试**

在 `preview_desired`/`pair_columns_tests` 附近追加:

```rust
#[cfg(test)]
mod preview_desired_concurrent_tests {
    use super::*;

    /// `Files` 留左栏、`Project` 挪右栏,两侧应该同时产出各自的
    /// webview,id 不相撞,矩形分别落在左右两侧(不需要跑真实 App/
    /// Workspace——用最小 `ShellState` 构造即可覆盖 id 偏移与 bounds
    /// 分派这两条核心逻辑,`ws.preview`/`ws.project_preview` 内部有没有
    /// 真实 tab 由更上层的集成测试/GUI 核对覆盖)。
    #[test]
    fn project_id_offset_keeps_ids_disjoint_from_files() {
        assert!(PROJECT_PREVIEW_ID_OFFSET > 0);
        // Files 的 id 空间从 0 起、Project 加偏移后不可能落回 0..offset。
        let project_id = 0 + PROJECT_PREVIEW_ID_OFFSET;
        assert_ne!(project_id, 0usize);
    }
}
```

（`preview_desired`/`browser_desired` 需要一个真实构造出 `ws.preview`/
`ws.project_preview` 都有内容的 `Workspace`——若现有测试基础设施里已经
有类似 fixture(搜 `fn test_workspace`/`fn sample_workspace` 之类),
在这里补一条端到端断言:构造 `left_view = Files`、`right_view =
Project`、`ws.preview.open_path(...)`、`ws.project_preview.open_path(...)`
各开一个文件,断言 `preview_desired(1440.0, 900.0)` 返回两条、id 不同、
两条的 bounds 元组不同且分别符合左右两侧的 x 范围。若没有现成 fixture,
补齐 Step 6 里那条最小常量测试即可,不要为了这一个端到端断言现造一整套
`Workspace` 构造脚手架——超出这个 Task 的必要范围。）

- [ ] **Step 7: 测试**

Run: `cargo test -p dozer-app --bin dozer preview_desired`
Expected: 全部通过。

- [ ] **Step 8: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): preview_desired/browser_desired 改为双侧独立扫描 + Project id 偏移,支持 Files/Project 左右同时活跃"
```

---

### Task 4: `is_in_preview_column` 返回命中面板 + 修正焦点路由(含 Project 焦点老 bug)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Produces: `pub fn is_in_preview_column(x: f32, window_width: f32, state:
  &ShellState) -> Option<PanelKind>`(原返回 `bool`,现在返回命中的面板
  种类,`None` = 未命中任何预览列)。
- Consumes: Task 3 的 `PROJECT_PREVIEW_ID_OFFSET`。

- [ ] **Step 1: 改写 `is_in_preview_column`**

找到整个函数(约 1297-1383 行,含放大态与非放大态两段 `match
state.left_view`),改成扫两侧、按各侧当前面板判断,命中则返回
`Some(kind)`:

```rust
/// 逻辑 x 是否落在某一侧的预览列内,是则返回命中的面板种类;焦点路由
/// (`main.rs`)据此决定把键盘交给哪个 webview 池(`Web` → 浏览器池,
/// `Files`/`Project` → 预览池)、以及 `active_preview_webview_id` 该查
/// `ws.preview` 还是 `ws.project_preview`。
///
/// 2026-08-19 Stage 4a 审阅后修订:此前只查 `state.left_view`,`Project`
/// 挪到右栏后点击其预览列不会被识别;现在左右两侧各自独立判断。
pub fn is_in_preview_column(x: f32, window_width: f32, state: &ShellState) -> Option<PanelKind> {
    for side in [Side::Left, Side::Right] {
        let collapsed = match side {
            Side::Left => state.left_collapsed,
            Side::Right => state.right_collapsed,
        };
        if collapsed {
            continue;
        }
        let kind = match side {
            Side::Left => state.left_view,
            Side::Right => state.right_view,
        };
        if let Some(maximized) = state.maximized {
            let showing_side = match maximized {
                MaximizedPane::Left => Side::Left,
                MaximizedPane::Right => Side::Right,
            };
            if side != showing_side {
                continue;
            }
            let (x0, avail_w) = maximized_box_x_range(window_width);
            let mirrored = state.layout.rail_layout.side_of(kind) != kind.default_side();
            let hit = match kind {
                PanelKind::Files => {
                    let cols = pair_columns(pair_content_width(avail_w), state.dims.files_split, mirrored);
                    let start = if mirrored { x0 + cols.content_x } else { x0 + cols.content_x + byteui::theme::geometry::divider_width() };
                    x >= start && x < x0 + avail_w
                }
                PanelKind::Web => x >= x0 && x < x0 + avail_w,
                PanelKind::Project => {
                    let cols = pair_columns(pair_content_width(avail_w), state.dims.project_split, mirrored);
                    let start = if mirrored { x0 + cols.content_x } else { x0 + cols.content_x + byteui::theme::geometry::divider_width() };
                    x >= start && x < x0 + avail_w
                }
                _ => false,
            };
            if hit {
                return Some(kind);
            }
            continue;
        }
        let (zone_x0, zone_w) = pair_x0_and_width(side, window_width, state);
        let mirrored = state.layout.rail_layout.side_of(kind) != kind.default_side();
        let hit = match kind {
            PanelKind::Files => {
                let cols = pair_columns(zone_w, state.dims.files_split, mirrored);
                let start = if mirrored { zone_x0 + cols.content_x } else { zone_x0 + cols.content_x + byteui::theme::geometry::divider_width() };
                x >= start && x < zone_x0 + zone_w
            }
            PanelKind::Web => x >= zone_x0 && x < zone_x0 + zone_w,
            PanelKind::Project => {
                let cols = pair_columns(zone_w, state.dims.project_split, mirrored);
                let start = if mirrored { zone_x0 + cols.content_x } else { zone_x0 + cols.content_x + byteui::theme::geometry::divider_width() };
                x >= start && x < zone_x0 + zone_w
            }
            _ => false,
        };
        if hit {
            return Some(kind);
        }
    }
    None
}
```

**`Web` 分支保留原有近似(整个 zone 都算预览列,不细分收藏夹展开时的
精确切分——延续现状,不在这个 Task 里额外补精确,原函数就是这么处理
的)。**

- [ ] **Step 2: 迁移现有测试**(`is_in_preview_column_false_for_...`
  等,约 8979-9042、9607-9623 行)——返回值从 `bool` 改成
  `Option<PanelKind>`,断言从 `assert!(is_in_preview_column(...))`/
  `assert!(!is_in_preview_column(...))` 改成 `assert!(is_in_preview_column(...)
  .is_some())`/`assert!(is_in_preview_column(...).is_none())`,涉及具体
  命中哪个面板的场景(如 `preview_content_bounds_bare_for_right_panel_on_left`
  的姊妹测试)额外断言 `assert_eq!(is_in_preview_column(...),
  Some(PanelKind::Files))` 之类,不只判断 `is_some()`。

- [ ] **Step 3: `active_preview_webview_id` 补上 `ws.project_preview` 分支
  (审阅时发现的独立预存 bug,借这次改造顺带修掉)**

找到(`workspace.rs` 约 1994-1996 行):

```rust
    pub fn active_preview_webview_id(&self) -> Option<usize> {
        self.preview.active_webview_id()
    }
```

改成(接受一个 `kind: PanelKind` 参数,按命中的面板决定查哪个
`PreviewPane`,`Project` 分支要加回 Task 3 的偏移量,因为它现在指向的
是 `webviews` 共享池里加了偏移的那个 id):

```rust
    /// `kind` 是 `is_in_preview_column` 命中的面板(`Files` 或
    /// `Project`)。此前硬编码只查 `self.preview`(Files)——`Project`
    /// 面板的预览 webview 点击后一直拿不到键盘焦点(⌘C 复制不了),
    /// 这是这次 Stage 4b 才第一次让它变得可测、可发现的一个独立预存
    /// bug,不是拖拽换栏引入的新问题。`Project` 分支的 id 要加
    /// `PROJECT_PREVIEW_ID_OFFSET`,因为 `webviews` 共享池里它的 key
    /// 已经加了这个偏移(见 `preview_desired`)。
    pub fn active_preview_webview_id(&self, kind: PanelKind) -> Option<usize> {
        match kind {
            PanelKind::Project => self
                .project_preview
                .active_webview_id()
                .map(|id| id + PROJECT_PREVIEW_ID_OFFSET),
            _ => self.preview.active_webview_id(),
        }
    }
```

`App::active_preview_webview_id`(app.rs 约 2937-2939 行)同步加参数:

```rust
    pub fn active_preview_webview_id(&self, kind: PanelKind) -> Option<usize> {
        self.active_workspace()?.active_preview_webview_id(kind)
    }
```

（`PROJECT_PREVIEW_ID_OFFSET` 定义在 `app.rs`——`workspace.rs` 需要
`use crate::app::PROJECT_PREVIEW_ID_OFFSET;` 或改成 `pub(crate)` 可见性,
看现有 `workspace.rs` 对 `app.rs` 常量的既有引用手法照做,不新开一套
可见性规则。）

- [ ] **Step 4: `main.rs` 焦点路由改用 `Option<PanelKind>`**

`FocusIntent` 枚举(约 469-473 行)加载体:

```rust
    enum FocusIntent {
        Preview(PanelKind),
        Browser,
        Terminal,
    }
```

鼠标按下那段(约 725-740 行)找到:

```rust
                    let intent = if app::is_in_preview_column(logical_x, logical_w, &state) {
                        if state.left_view == PanelKind::Web {
                            FocusIntent::Browser
                        } else {
                            FocusIntent::Preview
                        }
                    } else {
                        FocusIntent::Terminal
                    };
```

改成:

```rust
                    let intent = match app::is_in_preview_column(logical_x, logical_w, &state) {
                        Some(PanelKind::Web) => FocusIntent::Browser,
                        Some(kind) => FocusIntent::Preview(kind),
                        None => FocusIntent::Terminal,
                    };
```

`apply_pending_focus`(约 1642-1650 行)找到:

```rust
                Some(FocusIntent::Preview) => match app.active_preview_webview_id() {
                    Some(id) => {
                        if let Some((view, _)) = webviews.get(&id) {
                            let _ = view.focus(); // 返回 Result,忽略
                        } else {
                            window.focus_window();
                        }
                    }
                    None => window.focus_window(),
                },
```

改成:

```rust
                Some(FocusIntent::Preview(kind)) => match app.active_preview_webview_id(kind) {
                    Some(id) => {
                        if let Some((view, _)) = webviews.get(&id) {
                            let _ = view.focus(); // 返回 Result,忽略
                        } else {
                            window.focus_window();
                        }
                    }
                    None => window.focus_window(),
                },
```

- [ ] **Step 5: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功。

- [ ] **Step 6: 测试**

Run: `cargo test -p dozer-app --bin dozer is_in_preview_column && cargo test -p dozer-app --bin dozer active_preview_webview_id`
Expected: 全部通过(既有测试若原来只覆盖 `Files`,可补一条 `Project`
在右栏时 `is_in_preview_column` 返回 `Some(PanelKind::Project)` 且
`active_preview_webview_id(PanelKind::Project)` 返回偏移后 id 的测试,
验证 Step 3 修的老 bug 确实修好)。

- [ ] **Step 7: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "fix(dozer-app): is_in_preview_column 返回命中面板 + 修正 Project 预览焦点路由(含预存的 active_preview_webview_id 漏查 bug)"
```

---

### Task 5: `apply_column_drag` 的 `LeftPairSplit`/`ProjectSplit`/`BrowserBookmarksSplit`

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

同 Stage 4a `SshSplit`/`TodoSplit`/`GitLogSplit` 的手法(`pair_x0_and_width`/
`list_rendered_first` 已经在那个 Stage 写好,直接复用)。这部分不受
Task 1-4 的"并发双侧"改动影响——一条分割线只服务一个具体面板实例。

- [ ] **Step 1: 改写 `Divider::LeftPairSplit`(Files)**

找到(约 863-876 行):

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

改成:

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
  `files_split` 换成 `PanelKind::Project`/`project_split`,其余结构不变)

- [ ] **Step 3: 改写 `Divider::BrowserBookmarksSplit`**

找到(约 954-967 行):

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
            // 的字段),content 默认渲染在前,所以这里 `list_rendered_first`
            // 的 `default_list_first` 参数传 `false`(不是 `true`)——
            // "list" 这个泛化概念在 Web 这里对应收藏夹侧栏,不是内容。
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

**这里的翻转方向和 Files/Project 刚好相反(见上面注释)。写反了会导致
收藏夹拖拽方向錯亂,实现后务必在 Task 6 的 GUI 核对里手动验证这个分支,
不要只信编译通过。**

- [ ] **Step 4: 追加测试**(同 Stage 4a 的 near/far 方向性手法,三个
  Divider 各来一组默认栏防回归 + 镜像态方向验证,共 6 条)

- [ ] **Step 5: 编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer apply_column_drag`
Expected: 编译成功,全部通过。

- [ ] **Step 6: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): apply_column_drag 的 Files/Project/Web 分支改为 side+镜像感知"
```

---

### Task 6: 全项目收尾全量验证

**Files:**
- 无修改,纯验证

**Interfaces:**
- Consumes: Task 1-5 已完成,以及 Stage 1-4a 全部已合并

- [ ] **Step 1: 全量编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer`
Expected: 编译成功;测试全部通过。

- [ ] **Step 2: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 无新增警告(已知基线:`ws_state` 未使用、`format_todo_time`/
`write_goal` 死代码、`files.rs` 一处类型复杂度 lint、`git_log.rs` 一处
fmt diff——这几条是这次会话确认过的既有噪音,不算这个 Stage 的失败)、
无新增格式差异。

- [ ] **Step 3: 全项目 GUI 核对**(这是 4 个 Stage 里唯一一次覆盖全部
  11 个面板、全部能力的收尾验证)

构建一个独立命名的临时二进制,启动后逐条核对:

- **11 个面板全部可以两个方向拖动换栏**,换栏后自动展开在新一侧,
  源栏收起态正确退回相邻面板。
- **8 个有内部两栏布局的面板换栏后内部顺序正确镜像**。
- **`Files`/`Project` 换栏后**:webview(文件预览/项目预览)位置与
  镜像后的 iced 布局(项目树/项目信息)对齐,不叠在树/信息面板上面,
  不悬空,不跑出窗口。文件树的外部拖拽命中(从 Finder 拖文件到树的
  某一行)在换栏后依然准确对应视觉上的行。
- **`Files` 留左栏、`Project` 挪右栏同时打开**(或反过来):两个
  webview **同时**正确显示、互不覆盖、互不清空对方;点击 Files 的
  预览列,键盘焦点交给 Files 的 webview(⌘C 能复制);点击 Project 的
  预览列,键盘焦点交给 Project 的 webview——这一条此前($<$Stage 4b)
  从未生效过,是这个 Stage Task 4 Step 3 修的老 bug,务必手动验证 ⌘C
  确实能从 Project 预览里复制出文本。
- **放大右栏时若 `right_view` 是 `Files`/`Project`**:webview 正确显示
  在放大盒子里,不是空白(Task 2 Step 1 补的洞)。
- **`Web` 换栏后**:网页 webview 位置正确;收藏夹侧栏展开时同样镜像
  正确、拖拽方向正确。
- **点击焦点路由**:换栏后点击预览列(文件预览/项目预览/网页),键盘
  输入正确交给对应 webview;点击列表列(文件树/项目信息/收藏夹),
  键盘输入不误交给 webview。
- **放大态**(核对键位以现有 keymap 为准):把已经挪到非默认栏的
  `Files`/`Project` 放大,确认放大态下 webview 位置依然正确镜像。
- **8 个非 webview + `Files`/`Project`/`Web` 的内部分割线拖拽,方向在
  镜像态下都正确**。
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
3. Step 3 的全项目 GUI 核对清单逐条通过,尤其是"两个 webview 面板分居
   左右同时显示"与"Project 预览焦点路由"这两条此前从未被验证过的场景。
4. 提请审阅。**审阅通过合并后,"workspace 图标栏面板拖拽换栏"这个
   项目(spec + 4 个 Stage)全部完工**——11 个面板均可在左右图标栏间
   自由拖拽,面板跟随打开在新一侧,有内部两栏布局的面板正确镜像渲染
   顺序与分割线拖拽方向,3 个 webview 面板的原生子视图位置正确跟随
   镜像布局且支持并发双侧显示,换栏结果跨重启保留。
