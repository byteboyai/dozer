# Git Log 面板扩展化试点设计

**状态:已批准(brainstorming 会话,2026-08-07)**

## 背景

`crates/dozer-app/src/workspace.rs` 已膨胀到 11000+ 行,一个巨型 `Message` enum 里
`SelectTab`/`PreviewSelectTab`/`BrowserSelectTab`/`GitLogSelectCommit` 等各面板的消息平铺
并列,而非各自独立。此前已有构想:把 rail 上每个图标对应的面板(Git Log、Preview、Browser、
Todo…)抽成自洽的"扩展"单元(自己的 `Message` + 状态 + `update` + 渲染),`workspace.rs` 内核
只保留 rail 管理、焦点切换、面板间编排。用户当时明确表态:先人工验证功能、逐步剥离非核心
面板,不主动发起大重构;真要推进时,先做单一面板的技术验证,把消息路由方案跑通再铺开。

用户这次明确要求开始推进。同时"浏览器收藏夹"功能(见
`docs/superpowers/specs/2026-08-07-browser-bookmarks-design.md`)的 Task 6/7(把新状态/消息
接进 `Workspace`、把 UI 接进 `browser_pane`)尚未落地,而这正是要剥离的 `Workspace::browser`
那块代码——已确认排期:先让收藏夹功能完工,这次只做 **Git Log 面板** 一个试点,browser 等
收藏夹落地后再单独走同一套流程。

## 目标 / 非目标

**目标**:
1. `git_log` 模块拥有自己的 `Message` 枚举、`State` 结构体、`update` 函数、`view` 函数——
   不再直接引用顶层 `Message`,也不再被顶层 `Message` 直接内联处理。
2. 顶层 `Message` 只留一个包装变体 `Message::GitLog(extensions::git_log::Message)` 做转发;
   `App::update` 对应一支把大部分消息委托给 `extensions::git_log::update`。
3. `App` 上原本 6 个散装 `git_log_*` 字段合并成一个 `git_log: extensions::git_log::State`
   字段。
4. `git_log.rs` 迁到 `crates/dozer-app/src/extensions/git_log.rs`,新增
   `crates/dozer-app/src/extensions.rs` 作为该目录的模块入口——为后续 browser/todo 等面板的
   同类拆分预留位置,不用再挪一次目录。
5. "什么时候该刷新"(`sync_git_log_to_active_project`、`LeftIconSelect(GitLog)` 激活时机、
   `LoadMore` 需要的"当前项目路径")留在内核编排,不下放进 `git_log` 模块——`git_log` 模块
   不知道、也不该知道"哪个项目现在聚焦"这种跨面板知识。

**非目标**:
- **不建 `Extension` trait / 运行时注册表**——阶段 2 议题,这次是阶段 1(代码组织重构),
  静态分发,不引入 trait 对象。
- **不改 `git_log` 状态"`App` 级共享、不按项目分"的现状行为**——纯重构,行为不变(已在
  brainstorming 会话里跟用户确认:这是一个独立的产品/bug fix 决策,不跟这次试点混着做)。
- **不动 browser/todo/preview 等其他面板**——browser pane 等收藏夹 Task 6/7 落地后再单独
  走一轮同样的拆分。
- **不改 `sync_git_log_to_active_project` 的触发时机/条件**——只搬它调用的目标函数位置,
  判断逻辑本身不变。
- **不引入 `iced::Task`/`Command` 风格的异步返回值**——继续用现有
  `tokio::runtime::Handle::spawn` + 回调(`emit`)机制,与 `spawn_acceptance_count_refresh`
  等既有代码风格一致。
- **不拆独立 crate**——`git_log` 模块用到的 `iced_widget`/`theme`/`workspace_font` 都是
  `dozer-app` 内部模块,现在拆 crate 成本不小,且接口形状还没在实践里验证过;继续留在
  `dozer-app` 内,只是从平铺模块挪进 `extensions/` 子目录。

## 关键语义确认(brainstorming 会话定案)

- 路线:阶段 1(代码组织重构,模块自己的 Message+State+update,内核包装转发,静态分发)→
  未来成熟后再谈阶段 2(真正的 `Extension` trait + 注册表,甚至独立 crate)。这次只交付
  阶段 1,且阶段 1 也只落地 Git Log 一个试点。
- `extensions/` 目录名的选择:讨论过 `pane/`/`panels/`,最终定 `extensions/`——因为要拆的
  不只是"UI 面板"这一面,而是"自己的 Message+State+update+view 自洽整体",提前用阶段 2
  的目的地名字,以后升级成真正的 `Extension` trait 时这些模块天然就在该在的位置。
- **"内核转发"不是对所有消息都无差别的盲转发**:`git_log::Message::LoadMore` 需要"当前
  聚焦项目的仓库路径"才能执行,而这是 `App`(内核)才知道的跨面板知识,`git_log` 模块不该
  持有 `Workspace`/`active_project_path()` 这类概念。因此 `LoadMore` 在内核的 `match` 里
  单独一支处理(查路径 + 调用 `git_log::request_refresh`),其余三种消息(`SelectCommit`/
  `DetailLoaded`/`SnapshotLoaded`)才走统一的 `extensions::git_log::update` 转发。这是一个
  刻意的例外,不是遗漏——写实现计划/代码时要如实保留这个不对称,不能为了"看起来更纯粹"
  而把路径查找逻辑硬塞进 `git_log` 模块。
- `SnapshotLoaded` 落地后如果有"加载更多"发起前记下的 `restore_after_load` 提交,需要级联
  发起一次 `SelectCommit`——现有代码(`workspace.rs` 里 `GitLogSnapshotLoaded` 分支尾部)是
  直接 `self.update(Message::GitLogSelectCommit(oid))` 递归调用。拆分后镜像同一模式:
  `extensions::git_log::update` 返回 `Option<extensions::git_log::Message>` 表示"还有一条
  自己产生的后续消息要处理",内核收到 `Some(next)` 时 `self.update(Message::GitLog(next))`
  递归分发——与 `BrowserAddrEvent` 处理里"地址栏回车解析出 URL 后 `self.update
  (Message::BrowserOpenUrl(url))`"是同一个既有模式,不是新发明的机制。

## 架构与数据流

### 1. 目录迁移

```
crates/dozer-app/src/git_log.rs  →  crates/dozer-app/src/extensions/git_log.rs
```

新增 `crates/dozer-app/src/extensions.rs`:

```rust
pub mod git_log;
```

`main.rs` 的 `mod git_log;` 删掉,改成按字母序插入 `mod extensions;`。

### 2. `extensions::git_log` 自己的 `Message`/`State`

```rust
pub enum Message {
    SelectCommit(git2::Oid),
    LoadMore,
    DetailLoaded(PathBuf, git2::Oid, Result<CommitDetail, String>),
    SnapshotLoaded(PathBuf, usize, Result<GitLogSnapshot, String>),
}

pub struct State {
    cache: Option<GitLogSnapshot>,
    error: Option<String>,
    selected: Option<git2::Oid>,
    detail: Option<Result<CommitDetail, String>>,
    pending: Option<(PathBuf, usize)>,
    restore_after_load: Option<git2::Oid>,
}

impl State {
    /// "加载更多"按钮的下一个 `max_count`:有缓存则在当前基础上
    /// `+ LOAD_MORE_STEP`,否则回落 `DEFAULT_MAX_COMMITS`。内核算 `LoadMore`
    /// 该传多大的 `max_count` 时调这个,不用自己知道 `LOAD_MORE_STEP` 常量。
    pub fn next_load_more_count(&self) -> usize {
        self.cache
            .as_ref()
            .map(|c| c.max_count() + LOAD_MORE_STEP)
            .unwrap_or(DEFAULT_MAX_COMMITS)
    }
}
```

字段语义与现在 `App` 上的 6 个字段逐一对应,文档注释原样搬迁。

### 3. `update`:处理三种"自给自足"的消息

```rust
/// 处理 `SelectCommit`/`DetailLoaded`/`SnapshotLoaded` 三种消息(`LoadMore`
/// 需要内核提供的仓库路径,不在这里处理,见内核侧 `Message::GitLog` 分支)。
/// 返回值:`Some(next)` = 这次处理还产生了一条要递归分发的后续消息(目前
/// 只有 `SnapshotLoaded` 落地后恢复选中提交这一种情况)。
pub fn update(
    state: &mut State,
    msg: Message,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Clone + Send + 'static,
) -> Option<Message> {
    match msg {
        Message::SelectCommit(oid) => {
            state.selected = Some(oid);
            state.detail = None;
            let Some(repo_path) = state.cache.as_ref().map(|c| c.repo_path().to_path_buf())
            else {
                return None;
            };
            handle.spawn(async move {
                let repo_path2 = repo_path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    crate::extensions::git_log::commit_detail(&repo_path2, oid)
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                emit(Message::DetailLoaded(repo_path, oid, result));
            });
            None
        }
        Message::DetailLoaded(repo_path, oid, result) => {
            let matches = state.selected == Some(oid)
                && state
                    .cache
                    .as_ref()
                    .map(|c| c.repo_path() == repo_path.as_path())
                    .unwrap_or(false);
            if matches {
                state.detail = Some(result);
            }
            None
        }
        Message::SnapshotLoaded(repo_path, max_count, result) => {
            let still_pending = state
                .pending
                .as_ref()
                .map(|(p, m)| (p.as_path(), *m))
                == Some((repo_path.as_path(), max_count));
            if !still_pending {
                return None;
            }
            state.pending = None;
            match result {
                Ok(snapshot) => {
                    state.cache = Some(snapshot);
                    state.error = None;
                }
                Err(err) => {
                    state.cache = None;
                    state.error = Some(err);
                }
            }
            state.restore_after_load.take().map(Message::SelectCommit)
        }
        Message::LoadMore => {
            unreachable!("LoadMore 由内核在 Message::GitLog 分支里直接处理,不会转发到这里")
        }
    }
}
```

### 4. `request_refresh`:内核发起刷新的入口

```rust
/// 现有 `spawn_git_log_refresh` 的搬家版本:清 `selected`/`detail`/
/// `restore_after_load`,记 `pending`,起 `spawn_blocking` 跑
/// `git_log::build`,完成后 `emit(Message::SnapshotLoaded(..))`。
pub fn request_refresh(
    state: &mut State,
    repo_path: PathBuf,
    max_count: usize,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Clone + Send + 'static,
) {
    state.selected = None;
    state.detail = None;
    state.restore_after_load = None;
    state.pending = Some((repo_path.clone(), max_count));
    handle.spawn(async move {
        let repo_path2 = repo_path.clone();
        let result =
            tokio::task::spawn_blocking(move || crate::extensions::git_log::build(&repo_path2, max_count))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
        emit(Message::SnapshotLoaded(repo_path, max_count, result));
    });
}
```

### 5. `view`

```rust
pub fn view(state: &State) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>
```

内部实现基本不变(现有 `view`/`detail_view`/`GitLogCanvas` 等直接搬过去),只是参数从五个
散装值(`snapshot`/`error`/`selected`/`detail`/`loading`)改成读 `&State`(`loading` 由
`state.pending.is_some()` 推导,不再单独传),返回类型从顶层 `Message` 换成
`extensions::git_log::Message`。

### 6. 内核侧(`workspace.rs`)改动

`App` 结构体:6 个 `git_log_*` 字段 → 一个字段:

```rust
git_log: extensions::git_log::State,
```

顶层 `Message` 删除 `GitLogSelectCommit`/`GitLogLoadMore`/`GitLogDetailLoaded`/
`GitLogSnapshotLoaded` 四个变体,加一个:

```rust
GitLog(extensions::git_log::Message),
```

`update()` 里原来四支分开处理的分支合并成两支(体现上面"`LoadMore` 例外"那条语义确认):

```rust
Message::GitLog(extensions::git_log::Message::LoadMore) => {
    let Some(path) = self.active_workspace().and_then(|ws| ws.active_project_path()) else {
        return;
    };
    let next = self.git_log.next_load_more_count();
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::GitLog(m));
    };
    extensions::git_log::request_refresh(&mut self.git_log, path, next, &handle, emit);
}
Message::GitLog(msg) => {
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::GitLog(m));
    };
    if let Some(next) = extensions::git_log::update(&mut self.git_log, msg, &handle, emit) {
        self.update(Message::GitLog(next));
    }
}
```

`Message::ProjectFsChanged` 分支里判断"是否值得重建 Git Log 快照"、调用刷新的那段逻辑,
`self.git_log_cache` 引用改成 `self.git_log.cache`(或加一个 `State::repo_path()` 访问器,
视写计划时哪种更顺手),调用目标从 `self.spawn_git_log_refresh(&repo_path, max)` 改成
`extensions::git_log::request_refresh(&mut self.git_log, repo_path, max, &handle, emit)`。

`sync_git_log_to_active_project` 保留在 `App impl` 里,内部同理把直接改字段/调用
`spawn_git_log_refresh` 的地方,改成调用 `extensions::git_log::request_refresh`。

`App::view()` 里 `LeftView::GitLog` 分支:

```rust
LeftView::GitLog => extensions::git_log::view(&app.git_log).map(Message::GitLog),
```

## 错误处理

不新增错误处理路径——现有的两条既有降级逻辑原样保留,只是挪了位置:
- `git_log::build`/`commit_detail` 失败时,`error`/`Err(result)` 走既有的红字展示。
- `DetailLoaded`/`SnapshotLoaded` 落地时核对"是否还对得上当前 `selected`/`pending`",对不上
  (项目已切走、或紧接着发起了另一次请求)就静默丢弃陈旧结果,不覆盖更新的状态。

## 测试策略

- `extensions/git_log.rs` 现有 6 组纯函数测试(`build`/`commit_detail`/geometry 相关)原样
  保留,搬文件不改测试内容。
- 新增 `update()`/`request_refresh()` 的状态机单测(不依赖真实 git 仓库、不依赖真实异步
  落地,直接构造 `State`/`Message` 断言纯状态转换,`handle`/`emit` 用一个记录调用次数但不
  真正跑异步的 dummy 处理,或者只测不触发异步分支的那部分分支——具体 harness 写法留给
  写计划时定):
  - `SelectCommit`:置 `selected = Some(oid)`、`detail = None`。
  - `DetailLoaded`:`selected`/`cache.repo_path()` 都匹配时写入 `detail`;任一不匹配时
    `state` 不变(两个反例分别测)。
  - `SnapshotLoaded`:`pending` 匹配时落地 `cache`/`error` 并清 `pending`;不匹配时整个
    `state` 不变。
  - `SnapshotLoaded` 落地后 `restore_after_load` 有值 → 返回值是
    `Some(Message::SelectCommit(oid))`;`restore_after_load` 为 `None` → 返回值是 `None`。
  - `State::next_load_more_count()`:有缓存时 `cache.max_count() + LOAD_MORE_STEP`;无缓存时
    `DEFAULT_MAX_COMMITS`。
- `workspace.rs` 侧:确认 `App` 结构体/`Message` 枚举改动后 `cargo build -p dozer-app` 干净
  通过即可,不为"纯转发"这两支单独加测试(与现有其余 `XxxLoaded` 转发分支的测试覆盖水平
  一致,即历来没有专门测)。
- 人工验收(`cargo run -p dozer-app`):打开一个 git 项目,进 Git Log 面板,确认提交图正常
  画出来、点提交能看详情、点"加载更多"能翻页且翻页后自动恢复之前选中的提交详情、切项目/
  切 tab 后面板内容跟着对(尤其"`App` 级共享缓存"这条现状行为要保持:切到另一个还没打开过
  Git Log 面板的项目,不应该报错或崩溃,行为应该跟改造前一致)。
- 编译期防回归:`cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p
  dozer-app --all-targets && cargo fmt --check` 全绿——这是纯重构最重要的安全网,行为不应该
  有任何肉眼可见的变化。

## 依赖变更

无新增依赖,无 `Cargo.toml` 改动(`git_log` 模块用到的 `git2`/`gleisbau` 等依赖原样跟着文件
搬进 `extensions/` 子目录,`dozer-app/Cargo.toml` 不变)。
