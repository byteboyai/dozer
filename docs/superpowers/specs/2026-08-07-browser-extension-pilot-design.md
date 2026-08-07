# 浏览器面板扩展化设计

**状态:已批准(brainstorming 会话,2026-08-07)**

## 背景

Git Log 面板已经按阶段 1(代码组织重构:自己的 `Message`/`State`/`update`/`view`,内核
`extensions::git_log::Message` 包装转发)完成拆分并通过审阅(见
`docs/superpowers/specs/2026-08-07-git-log-extension-pilot-design.md`)。浏览器面板是
memory 里记的第二个候选,此前因为收藏夹功能(全局+本项目两级书签)的 Task 6/7/8 还在飞、
会改到同一块 `Workspace`/`browser_pane` 代码而推迟——收藏夹功能现已完工并经审阅通过,阻塞
解除,这次启动浏览器面板的拆分。

用户已确认:收藏夹(状态、消息、UI)随浏览器一起划归 `extensions::browser`,不留在内核里
——收藏夹本来就是浏览器域专属,没有脱离浏览器 pane 存在的意义。

## 目标 / 非目标

**目标**:
1. `extensions::browser` 拥有自己的 `Message`/`State`/`update`/`view`,内核只留一个包装
   变体 `Message::Browser(extensions::browser::Message)` 做转发。
2. 收藏夹相关的状态(全局/项目收藏本地缓存、面板/菜单开合)、消息(增删查/开合)、UI 渲染
   函数(星标按钮、收藏菜单、收藏面板)全部并入 `extensions::browser`,不留任何 `Browser*`
   相关字段/消息/渲染函数在 `workspace.rs`。
3. 浏览器域的 tab/地址栏状态不再复用 `crate::preview::PreviewPane`,改用浏览器自己的、
   只认 URL 的精简类型(暂命名 `Tabs`)——砍掉 `TabKind::File`/`Acceptance`、`open_path`/
   `is_editable_extension`/`flyfish_url` 等浏览器用不到的文件预览专属逻辑。
4. `Workspace` 上原本 6 个 `browser*`/`bookmarks*` 字段合并成一个
   `browser: extensions::browser::State` 字段——**这份状态挂在每个 `Workspace` 上**(不像
   Git Log 挂在 `App` 上),项目切换靠 `Workspace` 自身的生命周期天然隔离,不需要 Git Log
   那种 `sync_git_log_to_active_project` 式的手动同步/清空逻辑。
5. 消息路由处理"点击驱动"(路由到当前聚焦项目)和"异步结果落地"(必须按消息自带的
   `ProjectId` 路由,不能按当前聚焦)两种不同需求——延续 Git Log 试点定的"内核 match 特案
   需要额外上下文的消息、其余走统一转发"这条原则,不引入第二个顶层包装变体。

**非目标**:
- **不改收藏夹的产品行为**——全局/项目两级、乐观本地更新+下次刷新纠正、去重规则(局部唯一
  索引)都原样保留,这次只挪代码位置。
- **不给 `Tabs` 引入比现在 `PreviewPane` 更强的能力**——纯粹是"砍掉浏览器用不到的分支"之后
  的等价物,不顺手加新功能(不做 tab 拖拽排序、不做多窗口)。
- **不动 `crate::preview`/`PreviewPane` 本身**——文件预览("Files",核心面板,不在剥离
  范围)继续用它,不受这次拆分影响。
- **不建 `Extension` trait/注册表**——跟 Git Log 试点一样是阶段 1,不是阶段 2。
- **不改 `dozer-app/src/bookmarks.rs` 的纯函数逻辑**——`bookmark_status`/`optimistic_add`/
  `optimistic_remove` 函数体不变,只挪文件位置(并入 `extensions/browser.rs`)。
- **不改 dozerd/协议层**——`BookmarkStore`/`Request::AddBookmark` 等 dozerd 侧、协议层的
  东西这次不碰,浏览器拆分纯粹是 `dozer-app` 内部的代码组织调整。

## 关键语义确认(brainstorming 会话定案)

- `PreviewPane` 不动,继续留在 `crate::preview` 作为文件预览专用的共享基础组件;浏览器改用
  自己独立的 `Tabs` 类型,不复用也不 fork 出一个"通用"抽象——两边各自维护、互不知情,允许
  以后独立演化(比如浏览器以后想加"多 tab 分组"之类的功能,不用担心影响文件预览)。
- 消息路由:单一 `browser::Message` 枚举,不拆成两个顶层包装变体。内核 `match` 里先特案
  `BookmarksLoaded(ProjectId, _)`/`BookmarksMutated(ProjectId, _)` 这两个异步结果变体(各自
  按自带的 `ProjectId` 走 `with_project`),其余变体(点击驱动)统一走
  `with_focused_project` + 通用转发给 `browser::update`。这是 Git Log 试点"`LoadMore` 例外,
  其余统一转发"这条原则的直接推广,只是这次有两个例外而不是一个。
- `browser::update`/`browser::view` 比 Git Log 对应函数多两个依赖:
  - `client: &Client`——收藏夹增删查要走 dozerd RPC,Git Log 只有本地 `git2` 阻塞调用,不
    需要网络。
  - `project_id: Option<i64>`——收藏夹按"全局/本项目"分组渲染、判定收藏状态都要知道"现在
    是哪个项目",`browser::State` 本身不存这个(避免状态冗余/漂移),内核每次调用时从
    `ws.project.as_ref().map(|p| p.id)` 现取传入,跟 Git Log 的 `LoadMore` 需要内核传
    `repo_path` 是同一个道理:凡是需要"当前项目/焦点"这类跨面板知识的地方,由内核在调用
    处提供,不下放进面板模块。
- 收藏夹的乐观本地更新/`emit` 回调机制原样沿用收藏夹功能里已经验证过的设计(见
  `docs/superpowers/specs/2026-08-07-browser-bookmarks-design.md`),这次只是换个持有位置,
  不重新设计。

## 架构与数据流

### 1. `Tabs`:浏览器专用的精简 tab/地址栏状态机

新类型,行为对齐现在 `PreviewPane` 被浏览器使用的那部分子集(不含文件/验收相关):

```rust
pub struct BrowserTab {
    pub id: usize,
    pub url: String,
    pub title: String,
}

pub struct Tabs {
    tabs: Vec<BrowserTab>,
    active: usize,
    next_id: usize,
    addr_editing: bool,
    addr_buffer: String,
}

impl Tabs {
    pub fn tabs(&self) -> &[BrowserTab];
    pub fn active_idx(&self) -> usize;
    /// 每次调用都新开一个 tab,不做去重——原样对齐现有 `PreviewPane::open_url` 的行为
    /// (`PreviewPane::open_path` 才有"同一文件已开则切过去"那条去重,`open_url` 没有)。
    /// 纯重构不改变这条现状,不顺手给浏览器加一条文件预览都没有的新去重逻辑。
    pub fn open_url(&mut self, url: String) -> usize;
    pub fn select(&mut self, idx: usize);
    pub fn close(&mut self, idx: usize);
    pub fn addr_editing(&self) -> bool;
    pub fn addr_buffer(&self) -> &str;
    pub fn addr_begin(&mut self);
    pub fn addr_text(&mut self, s: &str);
    pub fn addr_backspace(&mut self);
    pub fn addr_cancel(&mut self);
    /// 提交解析:`Ok(Some(url))` = 有效网址(无 scheme 自动补 `http://`);
    /// `Ok(None)` = 空输入,no-op;`Err(message)` = 本地路径(以 `/` 或 `~/` 开头),
    /// 浏览器不支持,`message` 就是"浏览器不支持打开本地文件"这条文案。`Tabs` 自己
    /// 不持有 `State.error`,用 `Result` 把"该不该报错、报什么"交还给调用方
    /// (`browser::update`)决定,不越界直接改外层状态。
    pub fn addr_submit(&mut self) -> Result<Option<String>, String>;
}
```

`addr_submit` 的字符串解析规则(无 scheme 补 `http://`、以 `/`/`~/` 开头判定为本地路径)
原样照抄现有 `PreviewPane::addr_submit`/`AddrTarget` 那段逻辑,只是把"返回 `AddrTarget::File`
交给调用方判断"改成直接在 `Tabs` 内部判定并通过 `Result` 表达,不改变任何判定条件本身。

### 2. `browser::Message`/`browser::State`

```rust
pub enum Message {
    OpenUrl(String),
    SelectTab(usize),
    CloseTab(usize),
    AddrClick,
    AddrEvent(AddrEvent),   // 复用现有 AddrEvent 类型(main.rs 键盘拦截层产生)
    TabScroll(bool),
    StarClick,
    BookmarkAdd(BookmarkScope),
    BookmarkRemove(i64),
    BookmarksToggle,
    BookmarksLoaded(ProjectId, Vec<BookmarkInfo>),
    BookmarksMutated(ProjectId, Result<(), String>),
}

pub struct State {
    tabs: Tabs,
    error: Option<String>,
    tab_first: usize,
    bookmarks: Vec<BookmarkInfo>,
    bookmarks_open: bool,
    star_menu_open: bool,
}
```

12 个变体对应现在顶层 `Message` 里的 12 个 `Browser*` 变体,逐一去掉 `Browser` 前缀搬过来;
字段语义与现在 `Workspace` 上对应的 6 个字段(`browser`/`browser_error`/`browser_tab_first`/
`bookmarks`/`browser_bookmarks_open`/`browser_star_menu_open`)一一对应。

### 3. `browser::update`

```rust
pub fn update(
    state: &mut State,
    msg: Message,
    project_id: Option<i64>,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::OpenUrl(url) => {
            state.error = None;
            state.tabs.open_url(url);
            state.tab_first = 0;
        }
        Message::SelectTab(idx) => state.tabs.select(idx),
        Message::CloseTab(idx) => {
            state.tabs.close(idx);
            state.tab_first = 0;
        }
        Message::AddrClick => {
            state.error = None;
            state.tabs.addr_begin();
        }
        Message::AddrEvent(ev) => match ev {
            AddrEvent::Text(s) => state.tabs.addr_text(&s),
            AddrEvent::Backspace => state.tabs.addr_backspace(),
            AddrEvent::Cancel => state.tabs.addr_cancel(),
            AddrEvent::Submit => match state.tabs.addr_submit() {
                Ok(Some(url)) => update(state, Message::OpenUrl(url), project_id, client, handle, emit),
                Ok(None) => {}
                Err(message) => state.error = Some(message),
            },
        },
        Message::TabScroll(right) => {
            // 逻辑照搬现有 `Message::BrowserTabScroll` 分支:一次翻 2 个 tab,
            // 上界不在此钳,渲染时 `tab_window` 钳制显示。
            if right {
                state.tab_first = state.tab_first.saturating_add(2);
            } else {
                state.tab_first = state.tab_first.saturating_sub(2);
            }
        }
        Message::StarClick => {
            state.error = None;
            state.star_menu_open = !state.star_menu_open;
        }
        Message::BookmarksToggle => state.bookmarks_open = !state.bookmarks_open,
        Message::BookmarkAdd(scope) => {
            state.star_menu_open = false;
            let Some(tab) = state.tabs.tabs().get(state.tabs.active_idx()) else {
                return;
            };
            let url = tab.url.clone();
            let title = tab.title.clone();
            let target_project_id = match scope {
                BookmarkScope::Global => None,
                BookmarkScope::Project => project_id,
            };
            let created_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            optimistic_add(&mut state.bookmarks, scope, target_project_id, &url, &title, created_ms);
            let Some(project_id) = project_id else { return };
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .add_bookmark(scope, target_project_id, &url, &title)
                    .await
                    .map_err(|e| e.to_string());
                emit(Message::BookmarksMutated(project_id, res));
            });
        }
        Message::BookmarkRemove(id) => {
            state.star_menu_open = false;
            optimistic_remove(&mut state.bookmarks, id);
            let Some(project_id) = project_id else { return };
            let client = client.clone();
            handle.spawn(async move {
                let res = client.remove_bookmark(id).await.map_err(|e| e.to_string());
                emit(Message::BookmarksMutated(project_id, res));
            });
        }
        Message::BookmarksLoaded(_, bookmarks) => state.bookmarks = bookmarks,
        Message::BookmarksMutated(_, res) => {
            if let Err(message) = res {
                state.error = Some(message);
            }
            // 无论成败都重新拉一次,纠正本地乐观更新(见 [[dozer-browser-bookmarks]]
            // 设计文档"数据流与状态机"一节的既定取舍,这次原样沿用)。
            request_bookmarks_refresh(project_id, client, handle, emit);
        }
    }
}
```

`BookmarksLoaded`/`BookmarksMutated` 的第一个字段(`ProjectId`)在内核路由阶段(见下一节)
已经用来决定"投给哪个 `Workspace`",传到这里后不需要再用——保留在 `Message` payload 里是
因为 `update` 函数签名统一处理所有变体,不为这两个变体单独抠掉已经路由过的字段(否则
`browser::Message` 要分裂成两种形状,增加认知负担,不值得为省两个字段做这个取舍)。

`emit` 不需要 `Clone`(跟 Git Log 试点一样):`match` 各分支互斥,每次 `update` 调用里
`emit` 最多被用一次——要么递归传给 `AddrEvent::Submit` 内部那次 `update` 调用,要么转手
交给 `BookmarksMutated` 分支的 `request_bookmarks_refresh`,要么在某个 `handle.spawn`
的 `async move` 块里被移进去,不会出现同一次调用里用两次的情况。

### 4. `browser::request_bookmarks_refresh`

```rust
/// 现有 `Workspace::spawn_bookmarks_refresh` 的搬家版本。
pub fn request_bookmarks_refresh(
    project_id: Option<i64>,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let Some(project_id) = project_id else { return };
    let client = client.clone();
    handle.spawn(async move {
        let bookmarks = client.list_bookmarks(Some(project_id)).await.unwrap_or_default();
        emit(Message::BookmarksLoaded(project_id, bookmarks));
    });
}
```

### 5. `browser::view`

```rust
pub fn view(state: &State, project_id: Option<i64>) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>
```

内部把现在 `browser_pane`/`current_browser_url`/`browser_star_button`/
`browser_bookmarks_toggle_button`/`bookmark_menu_row`/`browser_star_menu_popup`/
`bookmark_group`/`browser_bookmarks_panel` 这些渲染函数整体搬过来,签名从吃 `ws: &Workspace`
改吃 `state: &State`(+ 需要 `project_id` 的地方吃这个新参数),渲染逻辑不变,`Message` 类型
从顶层换成本模块的 `Message`。

### 6. 内核侧(`workspace.rs`)改动

`Workspace` 结构体:6 个字段 → 一个 `browser: extensions::browser::State`。

顶层 `Message`:删除 12 个 `Browser*` 变体,加 `Browser(browser::Message)`。

`update()` 三支(`emit` 闭包构造跟 Git Log 试点完全一致:`move |m| { let _ =
proxy.send_event(Message::Browser(m)); }`,三支各自 clone 一份 `proxy`):

```rust
Message::Browser(browser::Message::BookmarksLoaded(pid, bookmarks)) => {
    self.with_project(pid, move |ws, io| {
        let handle = io.handle.clone();
        let client = io.client.clone();
        let proxy = io.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Browser(m));
        };
        browser::update(
            &mut ws.browser,
            browser::Message::BookmarksLoaded(pid, bookmarks),
            Some(pid),
            &client,
            &handle,
            emit,
        );
    });
}
Message::Browser(browser::Message::BookmarksMutated(pid, res)) => {
    self.with_project(pid, move |ws, io| {
        let handle = io.handle.clone();
        let client = io.client.clone();
        let proxy = io.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Browser(m));
        };
        browser::update(
            &mut ws.browser,
            browser::Message::BookmarksMutated(pid, res),
            Some(pid),
            &client,
            &handle,
            emit,
        );
    });
}
Message::Browser(msg) => {
    self.with_focused_project(|ws, io| {
        let project_id = ws.project.as_ref().map(|p| p.id);
        let proxy = io.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Browser(m));
        };
        browser::update(&mut ws.browser, msg, project_id, &io.client, &io.handle, emit);
    });
}
```

（`BookmarksLoaded`/`BookmarksMutated` 落地后,`browser::update` 对应分支只是纯同步赋值/
读错误,不会真的再往下 spawn 异步任务——`BookmarksMutated` 分支例外,它会调用
`request_bookmarks_refresh` 发起新一轮拉取,所以这两支同样需要传入完整的
`client`/`handle`/`emit`,不能省略。）

`adopt_project`/`from_restore` 里现有的 `ws.spawn_bookmarks_refresh(io)` 调用点,改成调用
`browser::request_bookmarks_refresh(ws.project.as_ref().map(|p| p.id), &io.client, &io.handle, emit)`。

`App::view()` 的 `LeftView::Web` 分支:

```rust
LeftView::Web => browser::view(&ws.browser, ws.project.as_ref().map(|p| p.id)).map(Message::Browser),
```

## 错误处理

不新增错误处理路径,原样保留现有降级逻辑:
- 收藏夹增删失败 → `state.error` 展示文案(复用现有 `browser_error` 语义),乐观本地更新
  不回滚,下次 `request_bookmarks_refresh` 落地时纠正。
- 地址栏提交本地文件路径 → 报"浏览器不支持打开本地文件",不是错误路径的新增,只是从
  `workspace.rs` 的 `match` 分支挪进 `Tabs::addr_submit`/`browser::update`。

## 测试策略

- `Tabs` 的单测覆盖现在测试里跟 URL/tab 相关的部分(`open_url` 不去重、`select`/`close`、
  地址栏编辑态往返、`addr_submit` 对本地路径的拒绝),照抄 `preview.rs` 里对应测试的断言
  内容,只是换个类型名。
- `browser::update`/`request_bookmarks_refresh` 的单测,镜像 Git Log 试点的写法
  (`#[tokio::test]`,构造 `State` 喂 `Message` 断言状态转换;需要真实网络往返的分支
  ——`client.add_bookmark` 等——用现有 `Client` 是否有可 mock 的测试路径待写计划时确认,
  没有的话对齐现有 `dozer-client` "没有专门单测、靠协议层测试兜底"的既有覆盖水平)。
- 现有 `dozer-app/src/bookmarks.rs` 的 9 个纯函数测试原样保留,搬进
  `extensions/browser.rs` 内部或作为其私有子模块,测试内容不变。
- 人工验收:浏览器打开网页、tab 切换/关闭、地址栏输入(含本地路径报错)、星标增删、收藏
  面板开合、跨项目收藏隔离(全局收藏共享、本项目收藏各自独立)——对照收藏夹功能原来的验收
  清单(`docs/superpowers/plans/2026-08-07-browser-bookmarks.md` Task 8 Step 4)再走一遍,
  确认拆分没有改变任何可见行为。
- 编译期防回归:`cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p
  dozer-app --all-targets && cargo fmt --check` 全绿。

## 依赖变更

无新增依赖。
