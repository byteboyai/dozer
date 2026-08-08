# 首页落地页四栏布局重构设计

**状态:已批准(brainstorming 会话,2026-08-08)**

## 背景

首页落地页(点顶栏 Dozer 品牌页签进入,`AppPage::Home`)现状是一个静态两栏布局
(`home_page` → `row![home_sidebar, home_recents_column]`):左栏固定宽,画品牌行 +
"我的项目"最近项目列表 + "＋新增项目";右栏铺满剩余空间,并排画"最近的文件"/
"最近的对话"两张卡。这个布局和工作区主体(`left_icon_rail` + `left_panel_area` +
`right_panel_area` + `right_icon_rail` 四栏 + 图标切换)是两套不同的交互范式。

用户提出把首页改造成与工作区一致的 **四栏结构**
(`left_icon_rail` / `left_zone` / `right_zone` / `right_icon_rail`),
目的:一是提高首页的可扩展性(以后能像工作区一样往 rail 上加新 pane 图标),
二是让首页和工作区保持统一的交互方式,降低用户心智负担。

本次范围明确:左栏默认显示"项目列表" pane,右栏默认显示"浏览器" pane(复用
`extensions::browser`,不绑定任何项目,`project_id` 恒为 `None`——收藏夹走全局
作用域,`browser.rs` 已原生支持这个模式)。原"最近的文件"/"最近的对话"两卡合并成
一个"Recents" pane,挂在左侧 icon rail 上,默认不显示,点击图标才切入。

## 目标 / 非目标

**目标**:

1. `home_page` 改为 `row![home_left_icon_rail, home_left_zone, divider, home_right_zone, home_right_icon_rail]`,
   直接复用工作区已有的视觉 token(`theme::region::left_icon_rail`/
   `right_icon_rail`/`left_zone`/`right_zone`、`theme::geometry::icon_rail_width`、
   `divider_bar`、`rail_icon_button`、`HoverId::Rail` 悬停动画机制),不新建样式系统。
2. 新枚举 `HomeLeftView { ProjectList, Recents }`(默认 `ProjectList`)、
   `HomeRightView { Browser }`(默认且目前唯一 `Browser`,为将来扩展占位,呼应
   "以后再加 Todo/Files 等 pane"的既定方向)。语义、命名风格对齐既有
   `LeftView`/`RightView`。
3. `App` 新增字段:`home_left_view: HomeLeftView`、`home_right_view: HomeRightView`
   ——**不持久化**,不进 `ShellLayout`/`layout.json`,每次经 `Message::TopBarHome`
   进首页都重置为默认值(这是首页内部的临时导航态,不是项目态,没有"记住上次看的是
   哪个 pane"的需求);`home_browser: browser::State`——全新独立实例,不挂在任何
   `Workspace` 上,生命周期等于 App 自身。
4. 新消息:`Message::HomeLeftIconSelect(HomeLeftView)`、
   `Message::HomeRightIconSelect(HomeRightView)`、`Message::HomeBrowser(browser::Message)`
   (后者路由到 `browser::update(&mut self.home_browser, msg, None, &self.client,
   &self.handle, emit)`,`project_id` 固定传 `None`)。
5. `RailButton` 新增 3 个 variant:`HomeProjectList`、`HomeRecents`、`HomeBrowser`,
   复用现成的 `HoverId::Rail(RailButton::X)` + `app.hover_progress(..)` 悬停动画,
   不新建一套悬停机制。
6. **不做拖拽调宽、不做收起/放大**——`home_left_zone` 固定宽
   `theme::geometry::h0_sidebar_width()`(沿用原 `home_sidebar` 宽度),
   `home_right_zone` 为 `Length::Fill`。这个决定是本次范围裁剪的一部分:首页不需要
   工作区那套拖拽/持久化宽度的复杂度。
7. **ProjectList pane**(左栏默认,内容基本照搬现有 `home_sidebar`):搜索框占位、
   最近项目卡列表(`app.recent_projects` 前 5 条,空态"还没有项目")、"更多项目"
   占位、"＋新增项目"按钮(`Message::ProjectTabPickFolder`)——**去掉顶部 Dozer
   品牌行**(图标 + "Dozer" + 版本号):顶栏 `top_bar` 本身已有 `dozer_home_tab`
   品牌页签,这里再画一次属于视觉重复,去重后留出的空间给搜索框上移。
8. **Recents pane**(左栏可切换,原 `home_recents_column` 内容合并):原来"最近的
   文件"/"最近的对话"两卡并排(占整个右侧大空间),现在挤进较窄的固定宽
   `home_left_zone`,改成**上下堆叠**(`column![files_card, conversations_card]`
   替代 `row![..]`)。数据来源(`app.home_recent_files`/`home_recent_conversations`/
   `home_recents_loaded`)、异步加载时机(`Message::TopBarHome` 进入时立即
   `spawn_blocking` 拉取,与当前选中哪个 pane 无关,保证切到 Recents 时数据早已
   就绪)全部不变。
9. **Browser pane**(右栏默认):直接复用 `extensions::browser::view`/`update`,
   `project_id` 恒传 `None`。全局收藏夹(`BookmarkScope::Global`)正常工作;
   项目作用域收藏(`BookmarkScope::Project`)在无 `project_id` 时按现有逻辑
   no-op(`browser.rs` 已有对应测试覆盖这个路径)。

**非目标**:

- 不做首页 pane 选中态的跨会话持久化。
- 不做拖拽调宽/收起/放大——留给以后如果真有需求再单独设计。
- 不改变工作区主体(`left_icon_rail`/`left_panel_area`/`right_panel_area`/
  `right_icon_rail`、`LeftView`/`RightView`)的任何行为,首页的 `HomeLeftView`/
  `HomeRightView` 是独立的一套状态,不复用/不污染工作区已有枚举。
- 不把"项目列表""Recents"重构成拥有独立 `Message`/`State` 的 `extensions::` 模块
  ——两者都不需要自己的本地可变交互状态(项目列表纯读 `app.recent_projects`,发的
  是已存在的顶层 `Message::ProjectSelect`/`ProjectTabPickFolder`;Recents 纯读
  `app.home_recent_files`/`home_recent_conversations`,不接收任何消息),按现有
  `left_panel_area` 里 `LeftView::GitLog`(直接调 `git_log::view` 但那是有状态的
  extension)这类判断标准衡量,这两个 pane 目前更接近纯展示函数,保持普通 `fn`
  即可,不为了"看起来统一"而引入空转的 `Message`/`State` 骨架。
- 浏览器 pane 不新增"当前是否有活跃项目"的判断分支——首页的浏览器就是全局的,
  跟工作区里 `LeftView::Web`(绑定 `ws.project` 的那个)是两个独立实例,互不同步、
  互不影响。
- **不做 `App`/`Workspace` 拆分**:brainstorming 过程中讨论过要不要顺带把
  `workspace.rs`(现 7000+ 行,`App`/`Workspace` 两个 struct、`Message` 枚举、
  几十个视图函数混在一起)拆成 `app.rs` + `workspace.rs`。已与用户确认这是独立的
  大重构(`Message` 枚举归属、大量混合传参函数的拆分边界都需要单独定案),这次
  只拆 `homespace.rs`,`App`/`Workspace` 拆分留给以后单独一轮 brainstorming。

## 架构与数据流

### 0. 文件组织:独立 `homespace.rs`

首页相关代码从 `workspace.rs` 拆到新文件 `crates/dozer-app/src/homespace.rs`,
`main.rs` 加 `mod homespace;`。拆分边界比照仓库已有的 `preview.rs`/
`conversation.rs`/`layout.rs`——这几个都是"纯类型 + 纯函数"模块,不持有
`App`/`Workspace` 的 `impl` 块,`workspace.rs` 通过 `use crate::homespace::{..}`
引入后在 `App` 的方法里组装:

**移入 `homespace.rs`**:
- 类型:`HomeLeftView`、`HomeRightView`、`HomeRecentFile`、`HomeRecentConversation`。
- 纯函数:`load_home_recents`。
- 视图构建函数:`home_page`、`home_left_icon_rail`、`home_right_icon_rail`、
  `home_left_zone`、`home_right_zone`、`home_project_list_view`(原
  `home_sidebar`)、`home_recents_view`(原 `home_recents_column`)、
  `home_recent_files_card`、`home_recent_conversations_card`。这些函数签名不变
  (`fn xxx(app: &App, ..) -> Element<'_, Message, ..>`),只是从 `workspace.rs`
  搬到 `homespace.rs`,内部逻辑不改。

**留在 `workspace.rs`**:
- `App` 结构体本身(含 `home_left_view`/`home_right_view`/`home_browser`/
  `recent_projects`/`home_recent_files`/`home_recent_conversations`/
  `home_recents_loaded` 字段声明)——单一数据源仍是 `App`,跟 `PreviewPane`/
  `browser::State` 这些"类型定义在别处、实例字段仍声明在 `App`/`Workspace` 上"
  的既有先例一致。
- `Message` 枚举(顶层 `TopBarHome`/`HomeRecentsLoaded`/`HomeLeftIconSelect`/
  `HomeRightIconSelect`/`HomeBrowser` 变体不变,只是变体携带的 `HomeLeftView`/
  `HomeRightView` 类型改从 `crate::homespace` 导入)。
- `App::update` 里这 5 个消息的处理逻辑(第 3 节所述,直接改 `App` 私有字段,
  天然属于 `App` 自己的 `impl`,不下放)。

**可见性调整**(本次拆分带来的唯一"新增工作量",纯签名改动、不改行为):
`homespace.rs` 里的视图函数需要读 `App` 私有字段与调用 `workspace.rs` 里的
私有辅助函数/类型,以下改成 `pub(crate)`:
- `App` 字段:`recent_projects`、`home_recent_files`、`home_recent_conversations`、
  `home_recents_loaded`、`home_left_view`、`home_right_view`、`home_browser`、
  `daemon_error`。
- `workspace.rs` 里的辅助函数/类型:`rail_icon_button`、`divider_bar`、
  `zone_pane_border`、`relative_time_text`、`PaneCorner`。
- 均限定 `pub(crate)`(仓内可见),不导出到 crate 外部,不影响任何公共 API。
  `RailButton`/`HoverId`/`Divider`/`App::hover_progress`/`lh` 现状已是
  `pub`/`pub(crate)`,不需要改动(写计划时以实际代码为准复核一遍,这里只是
  brainstorming 阶段核对过的现状快照)。

这次**不**把项目列表/Recents 做成 `extensions::` 模块、**不**接入工作区自己的
`LeftView`/`RightView`(已跟用户确认:两者虽然只依赖 `App` 级数据、理论上通用,
但这次范围只做首页,以后若要在工作区内也能直接切出这两个面板,再单独立项——届时
因为不依赖任何 `Workspace` 级状态,升级成 extension 的改动成本低)。浏览器 pane
本身已经是 `extensions::browser`,首页这次只是复用它的第二份独立 `State`
(`app.home_browser`,`project_id` 恒 `None`),不需要额外改造。

### 1. 顶层布局

```rust
fn home_page(app: &App) -> Element<'_, Message, ..> {
    let body = row![
        home_left_icon_rail(app),
        home_left_zone(app),
        divider_bar(Divider::LeftRight, ..),
        home_right_zone(app),
        home_right_icon_rail(app),
    ]
    .height(Length::Fill);
    // 外层 column![body] + daemon_error 提示、外层 container/padding
    // 保持现有 home_page 的收尾逻辑不变。
}
```

`home_left_icon_rail`/`home_right_icon_rail` 结构镜像现有 `left_icon_rail`/
`right_icon_rail`(`rail_icon_button` + `MouseArea` hover + `container` 固定宽
`icon_rail_width()`),区别只是按钮列表变成 2 个(ProjectList/Recents)和 1 个
(Browser),且**点击已选中的图标不触发收起**(首页没有 collapse 概念,恒有一个
pane 显示)。

### 2. 状态类型

```rust
/// 首页左栏当前显示哪个 pane。语义、命名对齐工作区 LeftView,但这是独立枚举
/// ——首页导航态不与工作区共用。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum HomeLeftView {
    #[default]
    ProjectList,
    Recents,
}

/// 首页右栏当前显示哪个 pane。目前只有 Browser 一个 variant,为将来扩展占位。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum HomeRightView {
    #[default]
    Browser,
}
```

`App` 新增字段(紧邻现有 `home_recent_files`/`home_recent_conversations`/
`home_recents_loaded` 一起声明):

```rust
home_left_view: HomeLeftView,
home_right_view: HomeRightView,
/// 首页全局浏览器状态,不挂在任何 Workspace 上;view/update 调用时
/// project_id 恒传 None。
home_browser: browser::State,
```

`RailButton` 新增:

```rust
HomeProjectList,
HomeRecents,
HomeBrowser,
```

### 3. `Message` 与路由

```rust
HomeLeftIconSelect(HomeLeftView),
HomeRightIconSelect(HomeRightView),
HomeBrowser(browser::Message),
```

- `HomeLeftIconSelect(v)`/`HomeRightIconSelect(v)`:直接赋值
  `self.home_left_view = v`/`self.home_right_view = v`,无需像工作区
  `LeftIconSelect` 那样处理 collapse/GitLog 缓存清空之类的副作用。
- `HomeBrowser(msg)`:`browser::update(&mut self.home_browser, msg, None,
  &self.client, &self.handle, |m| Message::HomeBrowser(m))` ——与工作区
  `LeftView::Web`/`RightView`(若有)路由 `Message::Browser` 到 `ws.browser` 的
  写法一致,只是目标状态换成 `self.home_browser`、`project_id` 固定 `None`。
- `Message::TopBarHome` 处理器(workspace.rs:3491-3504)追加两行重置:
  `self.home_left_view = HomeLeftView::default();
  self.home_right_view = HomeRightView::default();` ——确保每次从工作区点回首页
  都回到默认视图,不残留上次切换的状态。

### 4. `view` 组装

- `home_left_zone(app)`:按 `app.home_left_view` 匹配,
  `ProjectList => home_project_list_view(app)`(即现有 `home_sidebar` 去掉品牌行
  后的内容,外层不再自带 `Length::Fixed(h0_sidebar_width())`——宽度由
  `home_left_zone` 外层容器统一控制),`Recents => home_recents_view(app, now_ms)`
  (即原 `home_recents_column` 内容,`row!` 改 `column!`)。
- `home_right_zone(app)`:按 `app.home_right_view` 匹配,目前只有
  `Browser => browser::view(&app.home_browser, None, Length::Fill,
  zone_pane_border(..)).map(Message::HomeBrowser)`。
- 两个 zone 容器套 `theme::region::left_zone()`/`right_zone()` 的背景/边框/margin
  token,与工作区 `left_panel_area`/`right_panel_area` 尾部收尾逻辑保持视觉一致
  (但不含 maximize/collapse 分支)。

### 5. 图标选择

`home_left_icon_rail` 两个按钮图标:ProjectList 复用 `icons::IconKind::Folder`
风格的"列表"类图标(写计划时从 Lucide 选,比如 `list`/`layout-list`,避免和工作区
`LeftView::Files` 的 `Folder` 图标完全撞脸造成用户误解两者是同一个 pane);Recents
用 `icons::IconKind::History` 或等价的"时钟/历史"类图标。`home_right_icon_rail`
的 Browser 图标复用工作区 `LeftView::Web` 已用的 `icons::IconKind::Globe`(同一
概念,视觉复用没问题——两处都是"浏览器"这个语义)。具体图标是否已存在于
`icons::IconKind` 由写计划时核对现状决定,不影响这里的架构决策。

## 错误处理

- `app.recent_projects` 为空 → ProjectList pane 画"还没有项目"兜底文案(现状行为
  不变)。
- `app.daemon_error` 存在 → 首页外层照常展示错误条(现状 `home_page` 尾部逻辑不变,
  不因四栏化受影响)。
- 首页浏览器(`home_browser`)打开 URL/加载失败 → 沿用 `extensions::browser`
  现有的 `state.error` 展示与 `AddrEvent` 编辑态处理,不需要新错误路径。

## 测试策略

- `HomeLeftIconSelect`/`HomeRightIconSelect` 处理器的单测:赋值正确性。
- `Message::TopBarHome` 处理器单测追加断言:重复进入首页后
  `home_left_view`/`home_right_view` 回到默认值(即使上次退出前手动切换过)。
- `home_browser` 路由的 `HomeBrowser` 消息单测:复用/参考现有
  `extensions::browser` 测试里 `project_id: None` 的用例(如
  `update_bookmark_add_project_scope_without_project_id_is_local_only`),确认
  首页浏览器打开 URL、全局收藏夹增删的行为符合预期。
- 人工验收:首页四栏视觉与工作区一致(rail 宽度、zone 背景/边框/margin 观感统一);
  左栏默认项目列表、点 Recents 图标能切到上下堆叠的两张卡且数据完整;右栏默认
  浏览器可用(能开 URL、能收藏);切工作区再切回首页,左右栏都回到默认 pane;
  项目列表为空/`daemon_error` 场景兜底文案不崩。

## 依赖变更

无新增依赖。
