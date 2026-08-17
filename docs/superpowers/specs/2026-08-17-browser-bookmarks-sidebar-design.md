# 浏览器面板收藏夹侧栏重构设计

**状态:已批准(brainstorming 会话,2026-08-17)**

## 背景

`extensions::browser`(`crates/dozer-app/src/extensions/browser.rs`)当前的收藏夹
UI 是一张"下拉卡片":点地址栏右侧的收藏夹图标(`bookmarks_toggle_button`),
`bookmarks_panel` 被 push 到 `content` 列的**地址栏下方**,以 `Length::Fill` 宽的
卡片形式出现,内容是"全局收藏"/"本项目收藏"两组扁平文字列表(纯文字标题,无图
标、无层级)。

用户给了参考草图(`dozer-paste-40a2eaeb...png`):收藏夹改成右侧常驻侧栏,与网页
内容左右分栏共存(不是盖在下方的下拉卡片),用文件夹图标区分"项目收藏"/"全局收
藏"两组,视觉上是一棵文件夹树(尽管实际不支持折叠,层级恒为两级)。

## 目标 / 非目标

**目标**:

1. 收藏夹从"地址栏下方的下拉卡片"改成"与网页内容左右分栏的常驻侧栏":点收藏夹
   图标开关侧栏显隐(复用现有 `bookmarks_open` 布尔,不新增开关状态),侧栏打开
   时网页内容区变窄让出空间,而不是盖住/顶掉网页内容。
2. 侧栏宽度可拖拽,且按项目持久化——新增 `PanelDims::browser_bookmarks_split:
   f32`,复用现有 `panel_layouts.json` 每项目持久化机制与 `Divider` 拖拽框架。
3. 收藏条目改用文件夹图标分组("项目收藏"/"全局收藏"各一个文件夹图标 + 标题),
   替换现有纯文字标题;组内条目样式保留现状(标题可点新开 tab、`×` 删除)。
4. 顶部 tab 栏与地址栏(含星标/收藏夹按钮)保持整行贯通、不参与左右分栏——分栏
   只发生在它们下方的内容区,与草图一致。
5. 网页内容的 wry webview 摆位几何(`app.rs::preview_content_bounds` 的
   `LeftView::Web` 分支)要跟着"收藏夹是否打开 + 侧栏占比"变化,否则真实网页会
   画在新侧栏的下面而不是让出空间。

**非目标**:

- 不改变收藏夹的数据模型(`BookmarkInfo`/`BookmarkScope`、`BookmarkAdd`/
  `BookmarkRemove`/`BookmarksLoaded`/`BookmarksMutated` 消息与落盘逻辑)。
- 不改变星标按钮/`star_menu_popup`("收藏当前页"弹出菜单)的交互与位置。
- 不支持"项目收藏"文件夹展开显示其他项目的收藏——侧栏恒为"当前项目"视角,与
  现有 `project_id` 过滤逻辑一致,`byteboy` 只是当前项目名当标题,不是可展开的
  子文件夹。
- 不支持文件夹折叠/展开——两个分组(项目收藏/全局收藏)恒展开,数量固定为 2,
  不引入折叠状态。
- 不改变"点收藏条目 = 新开 tab"的行为(维持 `Message::OpenUrl` 现状,不改成"当
  前 tab 内导航")。
- 不改变放大态(`MaximizedPane::Left` 时 `LeftView::Web` 分支)下的收藏夹交
  互——放大态下收藏夹侧栏禁用/隐藏(见"交互细节补充"),复杂度不值得为一个较少
  触达的态单独设计分栏。

## 架构与数据流

### 1. `PanelDims` 新增 `browser_bookmarks_split`

```rust
pub struct PanelDims {
    // ...既有字段...
    /// 浏览器面板配对:网页内容占左面板区宽度的比例,收藏夹侧栏(右)拿剩下的。
    /// 与其余四个 split 语义相反(内容占比而非列表占比)——浏览器是"内容在
    /// 左、收藏夹侧栏在右"的唯一左面板区配对,列表侧在右意味着 `apply_column_
    /// drag` 要像 `RightPairSplit` 那样把算出的 ratio 取反再写回,见下文。
    pub browser_bookmarks_split: f32,
}
```

`default_panel_dims()` 补一行 `browser_bookmarks_split:
theme::geometry::default_split_ratio()`,`sanitize_panel_dims` 补一行夹取(镜像
`git_log_split` 那条,防止磁盘上写坏的 0.0/1.0 让配对一侧消失)。

命名上这是**唯一**一个"split 存内容占比、列表在右"的左面板区字段——其余四个
(`files_split`/`project_split`/`ssh_split`/`todo_split`/`git_log_split`)都是"列
表在左、split 存列表占比"。选择让 `browser_bookmarks_split` 存"网页内容占比"而
不是"收藏夹占比",是为了让草图的"网页内容在左"视觉顺序与字段语义直接对应,读代
码时不用绕一层"这个字段其实是给右边那块用的"。

### 2. `Divider::BrowserBookmarksSplit` + `apply_column_drag`

```rust
pub enum Divider {
    // ...既有变体...
    /// 浏览器面板内部分割线:左边网页内容,右边收藏夹侧栏。
    BrowserBookmarksSplit,
}
```

`apply_column_drag` 新增分支。浏览器物理上在**左**面板区,但视觉顺序是"内容
左、列表右",与右面板区那两个配对(`RightPairSplit`)同构——不过 `RightPairSplit`
需要取反是因为它存的字段(`agent_split`/`conversations_split`)语义是"列表占
比",而 `browser_bookmarks_split` 语义是"内容占比"(见上一节的命名理由),两者
互补,所以这里的 `ratio`(拖拽点左侧、即内容侧的占比)可以直接写入,不需要取反:

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
    // ratio 是"拖拽点左侧"的占比 = 网页内容占比,直接对应
    // browser_bookmarks_split 的字段语义,不需要像 RightPairSplit 那样取反
    // ——取反需求只在"字段存的是列表占比"时才出现,这里字段本来就存内容占比。
    PanelDims {
        browser_bookmarks_split: ratio,
        ..state.dims
    }
}
```

"测试策略"一节的方向性单测直接验证这条推导——`logical_x` 越大(拖拽点越靠
右)网页内容区应该越宽,即 `browser_bookmarks_split` 应该越大;如果实现后单测/
手感与此相反,说明这处推导有误,应改成 `1.0 - ratio`,以实测为准。

### 3. `ShellState` 新增字段,`preview_content_bounds` 的 `LeftView::Web` 分支改配对公式

`ShellState` 新增:

```rust
pub struct ShellState {
    // ...既有字段...
    /// 浏览器收藏夹侧栏是否展开——`preview_content_bounds` 的 `LeftView::Web`
    /// 分支据此决定网页 webview 要不要让出侧栏宽度。`browser::State::
    /// bookmarks_open` 字段是私有的(`browser.rs:820`,无 `pub`),需要新增
    /// 一个只读访问器 `pub fn bookmarks_open(&self) -> bool`,`shell_state()`
    /// 通过它读取,不直接开字段可见性。
    pub browser_bookmarks_open: bool,
}
```

`App::shell_state()` 构造处补上这个字段(从 `active_workspace()` 的
`ws.browser` 读取;无活跃项目/无 workspace 时给 `false`)。

`preview_content_bounds` 的 `LeftView::Web` 非放大分支(`app.rs:873-879` 附近)从
单栏公式改成配对公式,复用 `pair_content_width`/`pair_list_content_width`(与
`LeftView::Files`/`LeftView::Project` 同款,但内容/列表顺序相反):

```rust
LeftView::Web => {
    let y = y_top(theme::geometry::browser_chrome_top_px());
    let h = h_for(y);
    let x = theme::geometry::icon_rail_width() + 8.0 + m.left;
    if state.browser_bookmarks_open {
        let pair_w = pair_content_width(left_w);
        let (_bookmarks_w, content_w) =
            pair_list_content_width(pair_w, 1.0 - state.dims.browser_bookmarks_split);
        // `pair_list_content_width` 签名是 (list_w, content_w) 且假设
        // "list 占比"在前——这里 browser_bookmarks_split 存的是内容占比,
        // 传参时用 1.0 - split 换算成"侧栏(list)占比"喂给它,取到的
        // content_w 就是网页要用的宽度。webview 只需要这个宽度,不需要
        // bookmarks_w(收藏夹侧栏是纯 iced 渲染,不挂 webview)。
        let w = (content_w - 16.0 - m.left - m.right).max(0.0);
        (x, y, w, h)
    } else {
        let w = (left_w - 16.0 - m.left - m.right).max(0.0);
        (x, y, w, h)
    }
}
```

放大态(`MaximizedPane::Left` 分支里的 `LeftView::Web`,`app.rs:817-823`)**不
变**——收藏夹侧栏在放大态下不可用(见"交互细节补充"),webview 继续占满整条放大
盒子。

### 4. `browser::view` 内容区改配对布局

现状(`view()` 尾部,`app.rs` 之外的 `browser.rs:1403-1434`):

```rust
let addr_row = row![addr, star_button(...), bookmarks_toggle_button(...)]...;
let mut content = column![tab_bar, tab_divider(), addr_row].spacing(region.gap);
if state.star_menu_open { content = content.push(star_menu_popup(...)); }
if state.bookmarks_open { content = content.push(bookmarks_panel(...)); }  // ← 删除这行
// ...error/空态...
container(content.padding(region.padding))...
```

改成:tab 栏 + 分隔线 + 地址栏依旧整行贯通、不参与分栏;`star_menu_popup` 位置
不变(它是从星标按钮弹出的浮层,与收藏夹侧栏是两回事,互不影响);**收藏夹侧栏
本身**从"push 进 `content` 列"改成包一层 `row![内容占位区, divider_bar, 收藏
夹侧栏]`,作为 `content` 列的最后一个元素,`Length::Fill` 撑满剩余高度:

```rust
let mut content = column![tab_bar, tab_divider(), addr_row].spacing(region.gap);
if state.star_menu_open {
    content = content.push(star_menu_popup(state, project_id));
}
if let Some(err) = &state.error { /* 不变 */ }

let body: Element<_> = if state.tabs.tabs().is_empty() {
    // 现状的"暂无网页"占位——不参与分栏,收藏夹开着也照样在整条内容区
    // 居中显示(空态下没有网页内容可让,分栏没有意义)。
    container(lh(text("暂无网页——在地址栏输入网址")...))
        .width(Length::Fill).height(Length::Fill).into()
} else {
    // 真实网页由 wry webview 叠加渲染,这里只需要一块透明占位(不能有
    // 不透明背景,否则会盖住 webview——同 `preview.rs` 里 webview 占位区
    // 的既有约定)。
    iced_widget::Space::new().width(Length::Fill).height(Length::Fill).into()
};

content = content.push(if state.bookmarks_open {
    // `bookmarks_split` 是新增的函数参数(内容占比,语义同
    // `PanelDims::browser_bookmarks_split`),见下方"跨层传参"。
    // `split_portions(split)` 返回 `(list, content)` 且入参是"list 占比",
    // 这里传 `1.0 - bookmarks_split` 换算成侧栏占比喂给它。
    let (list_portion, content_portion) = workspace::split_portions(1.0 - bookmarks_split);
    row![
        container(body).width(Length::FillPortion(content_portion)),
        divider_bar(Divider::BrowserBookmarksSplit, ..., ..., Message::ColumnDragStart(...)),
        bookmarks_side_panel(state, project_id).width(Length::FillPortion(list_portion)),
    ].height(Length::Fill).into()
} else {
    body
});
```

**跨层传参问题**:`browser::view()` 目前的签名是 `view(state: &State, project_id:
Option<i64>, width: Length, outer: Border)`,不接收 `PanelDims`。而
`browser_bookmarks_split` 存在 `App`/`Workspace` 层的 `PanelDims` 里,`browser::
State` 自己不持有。需要给 `view()` 新增一个参数 `bookmarks_split: f32`,调用处
(`app.rs` 里渲染 `LeftView::Web` 的地方,与 `preview_pane`/`project_pane` 等同
级)从 `app.dims.browser_bookmarks_split` 取值传进去(`PanelDims` 挂在 `App`
上,不是 `Workspace`——同一函数里 `files_split`/`agent_split` 等既有 split 字段
都是 `app.dims.xxx_split` 这个访问路径,照抄)——这与 `agent_list_pane`/
`conversation_list_pane` 等函数接收 `Length::FillPortion` 而非自己算分割比例是
同一套惯例,照抄即可,不是新模式。

`divider_bar` 的具体调用参数(背景色两端、`Message::ColumnDragStart`)照抄
`RightPairSplit`/`ProjectSplit` 那几处的写法,不再展开。

### 5. `bookmark_group` 改文件夹图标分组

现状标题是纯文字 `lh(text(title)...)`。改成图标 + 文字一行,复用
`icons::view`(与 `agent_list_pane`/`home_panel_head` 同款图标绘制方式),图标
用现有 `icons::IconKind::Folder`(已存在,不新增图标资产)。组内条目(标题按钮 +
`×` 删除按钮)样式不变,只是整体在文件夹标题下缩进一级(比如 `padding{left:
16}` 或嵌套一层 `row![Space::new().width(16), col]`),视觉上呼应草图的树形缩
进。

`bookmarks_panel` 外层容器不再需要 `bookmarks_panel` 现状那套"浮层卡片"背景
(`theme::color::CARD` + 圆角边框,专为"盖在内容上方的下拉卡片"设计)——既然改
成了结构性的常驻侧栏,背景应改用与 `agent_list_pane`/`conversation_list_pane`
一致的列表侧背景(`#0a0e16`,即 `theme::region::browser_pane()` 复用的那套,或
新增一个 `browser_bookmarks_pane` region 条目,复用现成 `browser_pane` 更省事,
除非视觉验收时发现两者需要不同的 padding/gap)。

## 交互细节补充

- **放大态**:`MaximizedPane::Left` 且 `left_view == LeftView::Web` 时,收藏夹
  侧栏功能性隐藏——`bookmarks_toggle_button` 在放大态下点击应无效(或者干脆放
  大态下按钮本身置灰,实现时二选一,倾向"点击 no-op 但按钮仍可见并保持
  `bookmarks_open` 状态在退出放大后复原",避免用户放大时手滑点开又什么都没发
  生的困惑感更小)。放大态渲染分支不接分栏逻辑,直接复用现状。
- **拖拽下限**:侧栏宽度不能拖到 0 或吃光网页内容区——复用现有
  `theme::geometry::min_split_ratio()`/`max_split_ratio()` 夹取,与其余四个
  split 同一套下限,不需要新常量。
- **收藏夹为空态**:侧栏打开但两组都没有收藏时,现状的"暂无收藏"占位文案继续
  展示在侧栏里(不是隐藏侧栏或收起分栏)。

## 错误处理

不涉及新的错误路径——收藏夹增删的错误处理(`BookmarksMutated(Err(..))` 走
`optimistic_remove` 回滚)保持现状不变。唯一新增的失败模式是几何计算的边界情
况(`pair_w <= 0.0`),已通过复用 `apply_column_drag`/`preview_content_bounds`
里既有的 `.max(0.0)`/提前 `return` 套路处理,不需要新的错误分支。

## 测试策略

- `apply_column_drag` 新分支:镜像现有 `apply_column_drag_updates_git_log_
  split_ratio` 一类的纯函数单测,断言拖拽后 `browser_bookmarks_split` 落在
  `[min_split_ratio(), max_split_ratio()]` 区间内,且方向感正确(`logical_x`
  越大,网页内容占比——即 `browser_bookmarks_split`——应该越大,不是越小;这
  一条直接验证"架构与数据流"第 2 节的推导,写测试时如果符号反了要按实测结果
  改公式,不要迁就本文档)。
- `sanitize_panel_dims` 新字段的夹取单测,镜像 `sanitize_panel_dims_clamps_
  git_log_split`。
- `preview_content_bounds` 的 `LeftView::Web` 新分支:镜像现有
  `preview_content_bounds_web_view_spans_whole_left_zone`,新增一条
  `bookmarks_open = true` 时的用例,断言返回宽度比"收藏夹关闭"时的宽度更窄
  (让出了侧栏空间),且窄的量与 `browser_bookmarks_split` 成比例。
- `bookmark_group` 纯视觉改动,不需要新增快照/单元测试(现有 `browser.rs`
  测试模块里关于收藏夹增删的测试——`BookmarkAdd`/`BookmarkRemove`/
  `optimistic_remove` 等——保持不动,视觉重排不影响这些消息流)。
- 人工验收:参照草图确认——(a) 收藏夹关闭时浏览器面板与改动前视觉一致;(b)
  打开收藏夹后网页内容变窄、收藏夹侧栏出现在右侧且不挡住网页;(c) 拖拽分割线
  能调整侧栏宽度且方向符合直觉;(d) 切换项目/重启 app 后侧栏宽度记忆生效;(e)
  文件夹图标 + 缩进的视觉效果与草图基本一致。

## 依赖变更

无新增 crate 依赖——`icons::view`/`Divider`/`PanelDims`/`pair_content_width`/
`pair_list_content_width` 均为项目现有基础设施的复用。
