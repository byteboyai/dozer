# 浏览器面板收藏夹侧栏重构 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把浏览器面板(`extensions::browser`)的收藏夹从"地址栏下方的下拉卡片"
改成"与网页内容左右分栏、宽度可拖拽并按项目持久化的常驻侧栏",收藏条目改用
文件夹图标分组,与用户提供的参考草图对齐。

**Architecture:** 新增 `PanelDims::browser_bookmarks_split`(内容占比语义,
按项目持久化)+ `Divider::BrowserBookmarksSplit`(`apply_column_drag` 新分
支)。`browser::view()` 内部自建 `row![网页内容占位, divider_bar, 收藏夹侧
栏]`(不是 app.rs 拼两个 pane 的旧模式,是 `extensions::git_log` 今天刚落地
的"扩展自己拼配对、拖拽消息经内核转发"新模式)。`app.rs::preview_content_
bounds` 的 `LeftView::Web` 分支跟着"收藏夹是否打开"改配对公式,让 wry
webview 让出侧栏宽度。

**Tech Stack:** Rust、iced 0.14(`iced_widget`/`iced_renderer`)。无新增
crate 依赖。

**Spec:** `docs/superpowers/specs/2026-08-17-browser-bookmarks-sidebar-design.md`

## Global Constraints

- **分支要求**:必须在独立 worktree/分支(建议分支名
  `feature/browser-bookmarks-sidebar`)完成,不直接在 `main` 上改;全部任务
  做完、测试通过后走代码审阅,审阅通过再合并回 `main`。开工前先用
  `superpowers:using-git-worktrees` 技能建好 worktree,下面所有任务默认在该
  worktree 里执行。**这条尤其要遵守**:本仓库当前有另一个自主开发 agent 也在
  直接改 `main`(近几小时内已发生过并发编辑),不建独立分支会撞车。
- ByteBoy2077 配色沿用 `crates/dozer-app/src/theme/color.rs` 现有 token
  (`BG`/`CARD`/`BORDER`/`CREAM`/`DIM`),不新增色值。
- GUI 只用 iced 0.14 生态,不引入新依赖。
- 收藏夹数据模型(`BookmarkInfo`/`BookmarkScope`、`BookmarkAdd`/
  `BookmarkRemove`/`BookmarksLoaded`/`BookmarksMutated`)不改动。
- "点收藏条目 = 新开 tab"(`Message::OpenUrl`)行为不改。
- 两个分组("项目收藏"/"全局收藏")恒展开,不引入折叠状态。
- 放大态(`MaximizedPane::Left` 且 `LeftView::Web`)下收藏夹侧栏禁用,该渲染
  分支保持现状不变,不接分栏逻辑。
- 每个任务做完都要跑一遍 `cargo build -p dozer-app`、涉及的单测、
  `cargo clippy -p dozer-app --all-targets`、`cargo fmt -p dozer-app`,确认
  编译通过、测试通过、无新增 clippy 警告再进入下一个任务——本仓库同时有其他
  agent 在改 `crates/dozer-app/src/extensions/todo.rs`,如果 build 因为那个
  文件报错,先用 `git status`/`git diff` 确认是否是并发改动导致的暂时性冲突,
  不要把无关文件的问题算进本次改动。

---

### Task 1: `PanelDims::browser_bookmarks_split` 字段(数据层)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
  - `struct PanelDims`(约第 322 行)
  - `fn default_panel_dims()`(约第 348 行)
  - `fn sanitize_panel_dims()`(约第 423 行)

**Interfaces:**
- Consumes: 无(新字段,纯新增)
- Produces: `PanelDims::browser_bookmarks_split: f32`——后续任务(Task 2 的
  `apply_column_drag`、Task 3 的 `preview_content_bounds`、Task 5 的
  `browser::view()` 调用处)都读写这个字段。

- [ ] **Step 1: 写失败的单测**

在 `app.rs` 测试模块(`mod tests`,文件末尾附近,与
`sanitize_panel_dims_clamps_git_log_split` 相邻处)新增:

```rust
#[test]
fn default_panel_dims_includes_browser_bookmarks_split() {
    let dims = PanelDims::default();
    assert_eq!(
        dims.browser_bookmarks_split,
        theme::geometry::default_split_ratio()
    );
}

#[test]
fn sanitize_panel_dims_clamps_browser_bookmarks_split() {
    let dims = PanelDims {
        browser_bookmarks_split: 5.0,
        ..PanelDims::default()
    };
    let sanitized = sanitize_panel_dims(dims);
    assert!(sanitized.browser_bookmarks_split <= theme::geometry::max_split_ratio());

    let dims = PanelDims {
        browser_bookmarks_split: f32::NAN,
        ..PanelDims::default()
    };
    let sanitized = sanitize_panel_dims(dims);
    assert_eq!(
        sanitized.browser_bookmarks_split,
        PanelDims::default().files_split
    );
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer -- default_panel_dims_includes_browser_bookmarks_split sanitize_panel_dims_clamps_browser_bookmarks_split`
Expected: 编译失败,`PanelDims` 没有 `browser_bookmarks_split` 字段/`E0063` 缺字段初始化。

- [ ] **Step 3: 加字段 + 默认值 + 夹取**

`struct PanelDims` 里,在 `conversations_split` 字段后面加:

```rust
    /// 对话配对:对话列表占右面板区宽度的比例，对话审阅拿剩下的。
    pub conversations_split: f32,
    /// 浏览器面板配对:网页内容占左面板区宽度的比例,收藏夹侧栏(右)拿剩下
    /// 的。与其余 split 字段语义相反(内容占比而非列表占比)——浏览器是
    /// "内容在左、收藏夹侧栏在右"的唯一左面板区配对,详见 spec 第 1 节命名
    /// 理由。
    pub browser_bookmarks_split: f32,
```

`fn default_panel_dims()` 里,在 `conversations_split:
theme::geometry::default_split_ratio(),` 后面加:

```rust
        conversations_split: theme::geometry::default_split_ratio(),
        browser_bookmarks_split: theme::geometry::default_split_ratio(),
```

`fn sanitize_panel_dims()` 里,在 `conversations_split:
clamp_split(d.conversations_split),` 后面加:

```rust
        conversations_split: clamp_split(d.conversations_split),
        browser_bookmarks_split: clamp_split(d.browser_bookmarks_split),
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer -- default_panel_dims_includes_browser_bookmarks_split sanitize_panel_dims_clamps_browser_bookmarks_split`
Expected: 2 passed。这一步会连带触发整个 crate 重新编译,如果报错且错误信息
指向 `PanelLayout`/`ShellLayout` 的 `#[serde(default)]` 结构——不需要额外处
理,`serde(default)` 靠 `Default for PanelDims` 自动补新字段,老
`panel_layouts.json` 缺这个字段时会自动落 `default_panel_dims()` 的值,不需
要手写迁移代码。

- [ ] **Step 5: 编译 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt -p dozer-app`
Expected: 全部无错误、无新增 clippy 警告。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "feat(browser): PanelDims 新增 browser_bookmarks_split 字段"
```

---

### Task 2: `Divider::BrowserBookmarksSplit` + `apply_column_drag` 拖拽公式

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
  - `enum Divider`(约第 454 行)
  - `fn apply_column_drag()`(约第 599 行)

**Interfaces:**
- Consumes: `PanelDims::browser_bookmarks_split`(Task 1)、既有
  `pair_content_width()`/`left_zone_width()`/
  `theme::geometry::{icon_rail_width, min_split_ratio, max_split_ratio}()`。
- Produces: `Divider::BrowserBookmarksSplit` 变体——Task 5 的
  `browser::view()`(经 `crate::app::Divider::BrowserBookmarksSplit`)、
  `App::update()` 里 `Message::ColumnDragStart` 分支都要用到这个变体名。

- [ ] **Step 1: 写失败的单测**

紧邻 `apply_column_drag_updates_git_log_split_ratio` 新增:

```rust
#[test]
fn apply_column_drag_updates_browser_bookmarks_split_ratio() {
    let state = test_state();
    let window_width = 1600.0;
    // 拖拽点在左面板区靠右侧,网页内容(拖拽点左侧)占比应偏大。
    let result = apply_column_drag(state, Divider::BrowserBookmarksSplit, window_width, 500.0);
    assert!(result.browser_bookmarks_split >= theme::geometry::min_split_ratio());
    assert!(result.browser_bookmarks_split <= theme::geometry::max_split_ratio());
}

#[test]
fn apply_column_drag_browser_bookmarks_split_direction_matches_content_side() {
    // 方向性回归:拖拽点越靠右,网页内容(左侧)占比应该越大——
    // browser_bookmarks_split 存的是内容占比,不是收藏夹占比,见 spec 第 2
    // 节推导。若这条断言失败,说明推导反了,应把 apply_column_drag 里的
    // `ratio` 改成 `1.0 - ratio` 再写回,不要改这条测试迁就实现。
    let state = test_state();
    let window_width = 1600.0;
    let near = apply_column_drag(state, Divider::BrowserBookmarksSplit, window_width, 100.0);
    let far = apply_column_drag(state, Divider::BrowserBookmarksSplit, window_width, 500.0);
    assert!(
        far.browser_bookmarks_split > near.browser_bookmarks_split,
        "near={} far={}",
        near.browser_bookmarks_split,
        far.browser_bookmarks_split
    );
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer -- apply_column_drag_updates_browser_bookmarks_split_ratio apply_column_drag_browser_bookmarks_split_direction_matches_content_side`
Expected: 编译失败,`Divider` 没有 `BrowserBookmarksSplit` 变体。

- [ ] **Step 3: 加 Divider 变体 + apply_column_drag 分支**

`enum Divider` 里,在 `RightPairSplit,` 前面加(顺序不重要,放这里是紧邻其
余左面板区分割线):

```rust
pub enum Divider {
    LeftRight,
    LeftPairSplit,
    ProjectSplit,
    SshSplit,
    TodoSplit,
    GitLogSplit,
    /// 浏览器面板内部分割线:左边网页内容,右边收藏夹侧栏。与其余左面板区
    /// 分割线不同的是配对顺序反了(内容在左、列表在右),所以
    /// `apply_column_drag` 这条分支直接写 `ratio`(拖拽点左侧占比 = 内容占
    /// 比),不需要像 `RightPairSplit` 那样取反。
    BrowserBookmarksSplit,
    RightPairSplit,
}
```

`fn apply_column_drag()` 里,在 `Divider::GitLogSplit => { ... }` 分支后面
(`Divider::RightPairSplit` 分支前面)加:

```rust
        Divider::BrowserBookmarksSplit => {
            let pair_w = pair_content_width(left_zone_width(window_width, &state));
            if pair_w <= 0.0 {
                return state.dims;
            }
            let ratio = ((logical_x - theme::geometry::icon_rail_width()) / pair_w).clamp(
                theme::geometry::min_split_ratio(),
                theme::geometry::max_split_ratio(),
            );
            PanelDims {
                browser_bookmarks_split: ratio,
                ..state.dims
            }
        }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer -- apply_column_drag_updates_browser_bookmarks_split_ratio apply_column_drag_browser_bookmarks_split_direction_matches_content_side`
Expected: 2 passed。如果第二条方向性测试失败,把 `apply_column_drag` 里
`browser_bookmarks_split: ratio,` 改成 `browser_bookmarks_split: 1.0 - ratio,`
再重跑,直到通过——以测试结果为准,这条分支的符号在写 spec 时是推导出来的,
没有实机验证过。

- [ ] **Step 5: 编译 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt -p dozer-app`
Expected: 无错误、无新增警告。(已核查过 `Divider` 在这个 crate 里只有
`apply_column_drag` 一处穷尽匹配,`Message::ColumnDragStart(divider) => {
self.dragging = Some(divider); }` 那处是泛型赋值不逐变体匹配,不会因为新增
变体报错;`RowDivider` 是另一个独立枚举,不受影响。)

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "feat(browser): 新增 Divider::BrowserBookmarksSplit 拖拽分支"
```

---

### Task 3: `ShellState::browser_bookmarks_open` + `preview_content_bounds` 几何

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
  - `struct ShellState`(约第 510 行)
  - `fn terminal_grid_state()`(约第 1133 行,用 `..state` 展开,不用改)
  - `impl App { pub fn shell_state() }`(约第 2825 行)
  - `fn preview_content_bounds()` 的 `LeftView::Web` 非放大分支(约第 873 行)
  - 测试模块 `fn test_state()`(约第 8280 行)
- Modify: `crates/dozer-app/src/extensions/browser.rs`
  - `impl State`(约第 39 行起,`bookmarks_open` 字段定义在约第 820 行)

**Interfaces:**
- Consumes: `PanelDims::browser_bookmarks_split`(Task 1)、
  `pair_content_width()`/`pair_list_content_width()`。
- Produces: `ShellState::browser_bookmarks_open: bool`、
  `browser::State::bookmarks_open(&self) -> bool`——Task 5 不直接用这两个(Task
  5 的 `browser::view()` 内部直接读 `state.bookmarks_open` 私有字段,同一模块
  内可见),但 `preview_content_bounds` 的新分支要用到
  `state.browser_bookmarks_open`,这是本任务自己消费自己产出。

- [ ] **Step 1: 写失败的单测**

紧邻 `preview_content_bounds_web_view_spans_whole_left_zone` 新增:

```rust
#[test]
fn preview_content_bounds_web_view_shrinks_when_bookmarks_open() {
    let closed = ShellState {
        left_view: LeftView::Web,
        browser_bookmarks_open: false,
        ..test_state()
    };
    let open = ShellState {
        left_view: LeftView::Web,
        browser_bookmarks_open: true,
        ..test_state()
    };
    let (_, _, w_closed, _) = preview_content_bounds(1440.0, 900.0, &closed);
    let (x_open, y_open, w_open, h_open) = preview_content_bounds(1440.0, 900.0, &open);
    assert!(
        w_open < w_closed,
        "收藏夹打开时网页内容应该让出侧栏宽度: w_open={w_open} w_closed={w_closed}"
    );
    // x/y/h 不受收藏夹开关影响——网页内容起点、高度不变,只是变窄。
    let (x_closed, y_closed, _, h_closed) = preview_content_bounds(1440.0, 900.0, &closed);
    assert_eq!(x_open, x_closed);
    assert_eq!(y_open, y_closed);
    assert_eq!(h_open, h_closed);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --bin dozer -- preview_content_bounds_web_view_shrinks_when_bookmarks_open`
Expected: 编译失败,`ShellState` 没有 `browser_bookmarks_open` 字段。

- [ ] **Step 3: browser::State 加只读访问器**

`browser.rs` 的 `impl State { ... }` 块里(与其余访问器方法放一起,比如
`bookmark_hover`/`addr_editing` 附近)加:

```rust
    /// 收藏夹侧栏当前是否展开——`app.rs::App::shell_state()` 读这个填
    /// `ShellState::browser_bookmarks_open`,几何计算据此决定网页 webview
    /// 是否要让出侧栏宽度。
    pub fn bookmarks_open(&self) -> bool {
        self.bookmarks_open
    }
```

- [ ] **Step 4: ShellState 加字段 + shell_state() 填值 + test_state() 补字段**

`struct ShellState` 里,在 `maximized` 字段前面加:

```rust
pub struct ShellState {
    pub layout: ShellLayout,
    pub dims: PanelDims,
    pub left_view: LeftView,
    pub left_collapsed: bool,
    pub right_view: RightView,
    pub right_collapsed: bool,
    /// 浏览器收藏夹侧栏是否展开——`preview_content_bounds` 的
    /// `LeftView::Web` 分支据此决定网页 webview 要不要让出侧栏宽度。
    pub browser_bookmarks_open: bool,
    pub maximized: Option<MaximizedPane>,
}
```

`impl App { pub fn shell_state() }` 里补上这个字段:

```rust
    pub fn shell_state(&self) -> ShellState {
        ShellState {
            layout: self.shell_layout,
            dims: self.dims,
            left_view: self.left_view,
            left_collapsed: self.left_collapsed,
            right_view: self.right_view,
            right_collapsed: self.right_collapsed,
            browser_bookmarks_open: self
                .active_workspace()
                .map(|ws| ws.browser.bookmarks_open())
                .unwrap_or(false),
            maximized: self.maximized,
        }
    }
```

测试模块的 `fn test_state()` 补上这个字段:

```rust
    fn test_state() -> ShellState {
        ShellState {
            layout: ShellLayout::default(),
            dims: PanelDims::default(),
            left_view: LeftView::Files,
            left_collapsed: false,
            right_view: RightView::Agent,
            right_collapsed: false,
            browser_bookmarks_open: false,
            maximized: None,
        }
    }
```

- [ ] **Step 5: 跑测试确认还是失败(预期中的失败,几何公式还没改)**

Run: `cargo test -p dozer-app --bin dozer -- preview_content_bounds_web_view_shrinks_when_bookmarks_open`
Expected: 现在能编译了,但断言 `w_open < w_closed` 失败——因为
`preview_content_bounds` 的 `LeftView::Web` 分支还没读这个新字段,`open`/
`closed` 两种情况算出来的宽度目前完全一样。

- [ ] **Step 6: 改 preview_content_bounds 的 LeftView::Web 非放大分支**

找到 `fn preview_content_bounds()` 里(放大分支**之后**、`match
state.left_view` 里)的这一段,把:

```rust
        // 浏览器(Web)是单栏,左图标栏右侧 + 左 margin + 8 起,占满左面板区。
        LeftView::Web => {
            let y = y_top(theme::geometry::browser_chrome_top_px());
            let h = h_for(y);
            let x = theme::geometry::icon_rail_width() + 8.0 + m.left;
            let w = (left_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
```

改成:

```rust
        // 浏览器(Web):收藏夹侧栏关闭时单栏占满左面板区;打开时网页内容
        // 让出右侧收藏夹侧栏的宽度(纯 iced 渲染,不挂 webview,几何计算
        // 不用管它)。
        LeftView::Web => {
            let y = y_top(theme::geometry::browser_chrome_top_px());
            let h = h_for(y);
            let x = theme::geometry::icon_rail_width() + 8.0 + m.left;
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

（放大态 `MaximizedPane::Left` 分支里的 `LeftView::Web` 保持不动,不接这段
逻辑。）

- [ ] **Step 7: 跑测试确认通过**

Run: `cargo test -p dozer-app --bin dozer -- preview_content_bounds_web_view_shrinks_when_bookmarks_open preview_content_bounds_web_view_spans_whole_left_zone`
Expected: 2 passed(改动没有破坏收藏夹关闭时的既有行为)。

- [ ] **Step 8: 跑全量相关测试 + 编译 + clippy + fmt**

Run: `cargo test -p dozer-app --bin dozer -- preview_content_bounds && cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt -p dozer-app`
Expected: 全部通过,无新增警告。

- [ ] **Step 9: 提交**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/extensions/browser.rs
git commit -m "feat(browser): ShellState 感知收藏夹开关,webview 几何让出侧栏宽度"
```

---

### Task 4: `bookmarks_panel` 接收显式宽度 + 文件夹图标分组

**Files:**
- Modify: `crates/dozer-app/src/extensions/browser.rs`
  - `fn bookmark_group()`(约第 1223 行)
  - `fn bookmarks_panel()`(约第 1261 行)
  - `fn view()` 里当前调用 `bookmarks_panel(state, project_id)` 那一行(约第
    1417 行,`content.push(bookmarks_panel(state, project_id));`)——本任务先
    改成传 `Length::Fill` 保持行为不变,Task 5 再整体替换这一段。

**Interfaces:**
- Consumes: `icons::view`(已存在,签名 `view(kind: IconKind, size: f32,
  color: Color) -> Element`)、`icons::IconKind::Folder`(已存在)。
- Produces: `bookmarks_panel(state: &State, project_id: Option<i64>, width:
  Length) -> Element<'_, Message, ..>`——Task 5 的 `view()` 用
  `Length::FillPortion(list_portion)` 调它。

- [ ] **Step 1: 改 bookmark_group 加文件夹图标 + 缩进**

现状:

```rust
fn bookmark_group<'a>(
    title: &'static str,
    items: &[&'a BookmarkInfo],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![lh(text(title)
        .size(theme::font::subtitle())
        .color(theme::color::DIM))]
    .spacing(2);
    for b in items {
        let open = button(lh(text(b.title.clone())
            .size(theme::font::body())
            .color(theme::color::CREAM)))
        .on_press(Message::OpenUrl(b.url.clone()))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::color::CREAM,
            ..button::Style::default()
        });
        let remove = button(lh(text("×")
            .size(theme::font::body())
            .color(theme::color::DIM)))
        .on_press(Message::BookmarkRemove(b.id))
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::color::DIM,
            ..button::Style::default()
        });
        col = col.push(
            row![open, remove]
                .spacing(4)
                .align_y(iced_widget::core::Alignment::Center),
        );
    }
    col.into()
}
```

改成(标题行加文件夹图标,条目行整体缩进 16px):

```rust
fn bookmark_group<'a>(
    title: &'static str,
    items: &[&'a BookmarkInfo],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let header = row![
        icons::view(icons::IconKind::Folder, icon_size::row(), theme::color::DIM),
        lh(text(title)
            .size(theme::font::subtitle())
            .color(theme::color::DIM)),
    ]
    .spacing(4)
    .align_y(iced_widget::core::Alignment::Center);
    let mut col = column![header].spacing(2);
    for b in items {
        let open = button(lh(text(b.title.clone())
            .size(theme::font::body())
            .color(theme::color::CREAM)))
        .on_press(Message::OpenUrl(b.url.clone()))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::color::CREAM,
            ..button::Style::default()
        });
        let remove = button(lh(text("×")
            .size(theme::font::body())
            .color(theme::color::DIM)))
        .on_press(Message::BookmarkRemove(b.id))
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::color::DIM,
            ..button::Style::default()
        });
        col = col.push(
            row![
                iced_widget::Space::new().width(Length::Fixed(16.0)),
                row![open, remove]
                    .spacing(4)
                    .align_y(iced_widget::core::Alignment::Center),
            ]
            .width(Length::Fill),
        );
    }
    col.into()
}
```

`icon_size` 已经在 `browser.rs` 顶部 `use crate::theme::icon_size;` 引入,
`icons` 模块也已经 `use crate::icons;` 引入,不需要新增 `use`。

- [ ] **Step 2: 改 bookmarks_panel 签名接收 width**

现状:

```rust
fn bookmarks_panel(
    state: &State,
    project_id: Option<i64>,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let global: Vec<&BookmarkInfo> = state
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Global)
        .collect();
    let project: Vec<&BookmarkInfo> = state
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Project && b.project_id == project_id)
        .collect();

    let both_empty = global.is_empty() && project.is_empty();
    let mut col = column![].spacing(6);
    col = col.push(bookmark_group("全局收藏", &global));
    if project_id.is_some() {
        col = col.push(bookmark_group("本项目收藏", &project));
    }
    if both_empty {
        col = col.push(lh(text("暂无收藏")
            .size(theme::font::subtitle())
            .color(theme::color::DIM)));
    }

    container(col)
        .padding(6)
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::CARD.into()),
            border: Border {
                color: theme::color::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}
```

改成(签名加 `width: Length` 参数,`.width(Length::Fill)` 换成
`.width(width)`,背景从"浮层卡片"换成与 `agent_list_pane` 等列表侧一致的
`#0a0e16`——`theme::color::BG`):

```rust
fn bookmarks_panel(
    state: &State,
    project_id: Option<i64>,
    width: Length,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let global: Vec<&BookmarkInfo> = state
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Global)
        .collect();
    let project: Vec<&BookmarkInfo> = state
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Project && b.project_id == project_id)
        .collect();

    let both_empty = global.is_empty() && project.is_empty();
    let mut col = column![].spacing(6);
    col = col.push(bookmark_group("全局收藏", &global));
    if project_id.is_some() {
        col = col.push(bookmark_group("本项目收藏", &project));
    }
    if both_empty {
        col = col.push(lh(text("暂无收藏")
            .size(theme::font::subtitle())
            .color(theme::color::DIM)));
    }

    container(col)
        .padding(6)
        .width(width)
        .height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::BG.into()),
            ..container::Style::default()
        })
        .into()
}
```

- [ ] **Step 3: 临时更新现有调用点(保持编译通过,不改行为)**

`fn view()` 里现有的:

```rust
    if state.bookmarks_open {
        content = content.push(bookmarks_panel(state, project_id));
    }
```

临时改成(Task 5 会把这整段替换成 split-row 逻辑,这里先补参数保证能编
译):

```rust
    if state.bookmarks_open {
        content = content.push(bookmarks_panel(state, project_id, Length::Fill));
    }
```

- [ ] **Step 4: 编译 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt -p dozer-app`
Expected: 无错误、无新增警告。

- [ ] **Step 5: 跑浏览器扩展现有测试,确认没有破坏收藏夹增删逻辑**

Run: `cargo test -p dozer-app --bin dozer -- extensions::browser::tests`
Expected: 全部通过——本任务没有碰 `BookmarkAdd`/`BookmarkRemove`/
`optimistic_remove` 等消息流,纯视觉改动不应该影响这些既有测试。

- [ ] **Step 6: 人工验收(可选但推荐)**

跑 `cargo run -p dozer-app`,打开一个项目,切到浏览器面板,点收藏夹图标——
下拉卡片仍在地址栏下方(本任务还没改布局位置),但标题从纯文字变成"文件夹
图标 + 文字",条目整体有一层缩进。确认视觉符合预期再继续。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/extensions/browser.rs
git commit -m "feat(browser): 收藏夹条目改文件夹图标分组,bookmarks_panel 接收显式宽度"
```

---

### Task 5: `browser::view()` 改配对布局 + 拖拽消息跨扩展转发

**Files:**
- Modify: `crates/dozer-app/src/extensions/browser.rs`
  - `pub enum Message`(约第 763 行)
  - `pub fn update()`(约第 1030 行起)
  - `pub fn view()`(约第 1306 行)
- Modify: `crates/dozer-app/src/app.rs`
  - `impl App { fn update(...) }` 顶层 `match`,`Message::Browser(...)` 相关
    分支(约第 3618-3628 行)
  - `LeftView::Web => browser::view(...)` 调用点(约第 6959 行)

**Interfaces:**
- Consumes: Task 1 的 `PanelDims::browser_bookmarks_split`、Task 2 的
  `Divider::BrowserBookmarksSplit`、Task 4 的 `bookmarks_panel(state,
  project_id, width)`、既有 `crate::app::{divider_bar, split_portions}`(若
  `split_portions` 不是 `pub(crate)` 需要核实,见 Step 3 备注)。
- Produces: `browser::view()` 新签名 `view(state: &State, project_id:
  Option<i64>, bookmarks_split: f32, width: Length, outer: Border) ->
  Element<'_, Message, ..>`——这是本次重构里唯一一个外部(app.rs)调用点会用
  到的签名变化,改完之后浏览器面板收藏夹侧栏功能完整可用。

- [ ] **Step 1: browser::Message 加 ColumnDragStart 变体**

`pub enum Message { ... }` 里加(放在末尾或任意位置,不影响其余变体):

```rust
    /// 收藏夹侧栏分割线开始拖:扩展发不了 app 级拖拽消息,由内核代发,见
    /// `App::update` 里 `Message::Browser(Message::ColumnDragStart)` 分支
    /// (同 `extensions::git_log::Message::ColumnDragStart` 的处理方式)。
    ColumnDragStart,
```

- [ ] **Step 2: browser::update() 加 no-op 分支**

`pub fn update(...)` 的 `match msg { ... }` 里加:

```rust
        Message::ColumnDragStart => {
            debug_assert!(
                false,
                "ColumnDragStart 由内核在 Message::Browser 分支里直接处理\
                 (转成 app 级拖拽消息),不会转发到这里"
            );
        }
```

- [ ] **Step 3: 改 view() 签名 + 内容区改配对布局**

先确认 `split_portions` 的可见性:

```bash
grep -n "fn split_portions" crates/dozer-app/src/workspace.rs
```

如果是 `pub(crate) fn split_portions`,在 `browser.rs` 顶部 `use` 块里加
`use crate::workspace::split_portions;`(与已有的 `use crate::workspace::{lh,
preview_tab_display_width};` 合并成一行 `use crate::workspace::{lh,
preview_tab_display_width, split_portions};`);如果不可见,就地在 `browser.rs`
里复制一份同样逻辑的私有函数(两行代码,不值得为此改动 `workspace.rs` 的可
见性)。

现状(`view()` 尾部):

```rust
    let mut content = column![tab_bar, tab_divider(), addr_row].spacing(region.gap);

    if state.star_menu_open {
        content = content.push(star_menu_popup(state, project_id));
    }
    if state.bookmarks_open {
        content = content.push(bookmarks_panel(state, project_id, Length::Fill));
    }

    if let Some(err) = &state.error {
        content = content.push(lh(text(format!("⚠ {err}"))
            .size(theme::font::body())
            .color(theme::color::RED)));
    }

    if state.tabs.tabs().is_empty() {
        content = content.push(
            container(lh(text("暂无网页——在地址栏输入网址")
                .size(theme::font::subtitle())
                .color(theme::color::DIM)))
            .width(Length::Fill)
            .height(Length::Fill),
        );
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}
```

改成:

```rust
    let mut content = column![tab_bar, tab_divider(), addr_row].spacing(region.gap);

    if state.star_menu_open {
        content = content.push(star_menu_popup(state, project_id));
    }
    if let Some(err) = &state.error {
        content = content.push(lh(text(format!("⚠ {err}"))
            .size(theme::font::body())
            .color(theme::color::RED)));
    }

    let body: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if state.tabs.tabs().is_empty() {
            container(lh(text("暂无网页——在地址栏输入网址")
                .size(theme::font::subtitle())
                .color(theme::color::DIM)))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            // 真实网页由 wry webview 叠加渲染,这里只需要一块透明占位
            // (不能有不透明背景,否则会盖住 webview)。
            iced_widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

    content = content.push(if state.bookmarks_open {
        let bg = region.background.unwrap_or(theme::color::BG);
        let (list_portion, content_portion) = split_portions(1.0 - bookmarks_split);
        row![
            container(body).width(Length::FillPortion(content_portion)),
            crate::app::divider_bar(
                crate::app::Divider::BrowserBookmarksSplit,
                bg,
                bg,
                Message::ColumnDragStart,
            ),
            bookmarks_panel(state, project_id, Length::FillPortion(list_portion)),
        ]
        .height(Length::Fill)
        .into()
    } else {
        body
    });

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}
```

再改 `pub fn view(` 的函数签名,加 `bookmarks_split: f32` 参数(放在
`project_id` 后面、`width` 前面):

```rust
pub fn view(
    state: &State,
    project_id: Option<i64>,
    bookmarks_split: f32,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
```

- [ ] **Step 4: app.rs 加拖拽消息转发分支**

在 `impl App { fn update(...) }` 顶层 `match` 里,找到现有的:

```rust
            Message::Browser(browser::Message::BookmarksLoaded(pid, bookmarks)) => {
```

这一组特化分支(往下几行是 `BookmarksMutated`/`DragHover`,再往下是兜底的
`Message::Browser(msg) => self.browser_message(msg)`),在**兜底分支之前**加
一条:

```rust
            Message::Browser(browser::Message::ColumnDragStart) => {
                self.update(Message::ColumnDragStart(Divider::BrowserBookmarksSplit));
            }
```

- [ ] **Step 5: app.rs 更新 browser::view() 调用点**

找到:

```rust
            LeftView::Web => browser::view(
                &ws.browser,
                ws.project.as_ref().map(|p| p.id),
                Length::Fill,
                zone_pane_border(zone, ac),
            )
            .map(Message::Browser),
```

改成:

```rust
            LeftView::Web => browser::view(
                &ws.browser,
                ws.project.as_ref().map(|p| p.id),
                app.dims.browser_bookmarks_split,
                Length::Fill,
                zone_pane_border(zone, ac),
            )
            .map(Message::Browser),
```

- [ ] **Step 6: 编译**

Run: `cargo build -p dozer-app`
Expected: 编译通过。如果报 `Message::Browser` 匹配处 "unreachable pattern"
或反过来 "non-exhaustive",检查 Step 4 新增分支是不是插在了兜底分支
`Message::Browser(msg) => self.browser_message(msg)` **之后**——必须在它之
前,Rust `match` 按顺序取第一个匹配,兜底分支排前面会把新分支吃掉导致
`debug_assert!(false, ...)` 在 debug build 里直接 panic。

- [ ] **Step 7: 跑浏览器/几何相关测试**

Run: `cargo test -p dozer-app --bin dozer -- extensions::browser::tests preview_content_bounds apply_column_drag`
Expected: 全部通过。

- [ ] **Step 8: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets && cargo fmt -p dozer-app`
Expected: 无新增警告。

- [ ] **Step 9: 人工验收**

跑 `cargo run -p dozer-app`,打开一个项目,切到浏览器面板:
1. 点收藏夹图标——收藏夹从地址栏下方的下拉卡片位置消失,改成出现在网页内容
   右侧的常驻侧栏,网页内容区域相应变窄。
2. 拖拽侧栏与网页内容之间的分割线——侧栏宽度跟着变化,方向符合直觉(往右拖
   网页变宽/侧栏变窄,往左拖反过来)。
3. 关掉项目重开(或重启 app)——侧栏宽度记住了上次拖拽的比例。
4. 侧栏里两组("全局收藏"/"本项目收藏")都能看到文件夹图标 + 缩进的条目;
   点条目新开 tab;点 `×` 能删除。
5. 双击左面板区放大——放大态下收藏夹侧栏不参与分栏(如果放大前开着,放大
   后表现见 spec"交互细节补充"一节,退出放大后侧栏状态复原)。

- [ ] **Step 10: 提交**

```bash
git add crates/dozer-app/src/extensions/browser.rs crates/dozer-app/src/app.rs
git commit -m "feat(browser): 收藏夹改为可拖拽常驻侧栏,替换原下拉卡片"
```

---

## 完成后

全部 5 个任务做完、每步测试通过、人工验收符合 spec 描述后:

1. 跑一遍完整校验:`cargo build --workspace && cargo test -p dozer-app --bin dozer && cargo clippy -p dozer-app --all-targets && cargo fmt --check -p dozer-app`。
2. 用 `superpowers:requesting-code-review` 走一遍代码审阅。
3. 审阅通过后按 `superpowers:finishing-a-development-branch` 合并回 `main`。
