# Git Log 扩展化试点 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Git Log 面板拆成自洽模块(自己的 `Message`/`State`/`update`/`view`),`workspace.rs` 内核只留一个包装变体转发——阶段 1 扩展化重构的第一个试点,纯重构、行为不变。

**Architecture:** `crates/dozer-app/src/git_log.rs` 整体搬到 `crates/dozer-app/src/extensions/git_log.rs`,新增 `crates/dozer-app/src/extensions.rs` 作为模块入口;`git_log` 模块新增自己的 `Message` 枚举 + `State` 结构体 + `update`/`request_refresh` 函数,`view` 签名从五个散装参数改吃 `&State`;`App` 上原本 6 个 `git_log_*` 字段合并成一个 `git_log: extensions::git_log::State` 字段,顶层 `Message` 只留 `GitLog(extensions::git_log::Message)` 一个包装变体。

**Tech Stack:** Rust workspace;iced 0.14(`iced_widget`,`canvas::Program`);`tokio::runtime::Handle::spawn` + 手工 `emit` 回调(不引入 `iced::Task`);`git2`/`gleisbau`(依赖不变)。

## Global Constraints

- 纯重构,不改变任何用户可见行为——尤其"`git_log` 状态是 `App` 级共享、不按项目分"这条现状,这次**不修**(独立的产品/bug fix 决策,已跟用户确认排期在外)。
- 不建 `Extension` trait / 运行时注册表(阶段 2 议题,这次不做)。
- 不拆独立 crate,`git_log` 继续是 `dozer-app` 内的模块,只是挪进 `extensions/` 子目录。
- 不引入 `iced::Task`/`Command` 风格异步返回值,继续用现有 `tokio::runtime::Handle::spawn` + 回调机制,与 `spawn_acceptance_count_refresh` 等既有代码风格一致。
- `Message::GitLog(git_log::Message::LoadMore)` 在内核 `match` 里单独处理(需要内核才知道的"当前聚焦项目路径"),其余消息(`SelectCommit`/`DetailLoaded`/`SnapshotLoaded`)走统一的 `extensions::git_log::update` 转发——这是刻意的例外,不是遗漏,写代码时如实保留。
- 每个任务结束都要 `cargo build -p dozer-app && cargo test -p dozer-app` 干净通过。

---

### Task 1: 纯文件搬家——`git_log.rs` → `extensions/git_log.rs`

**Files:**
- Move: `crates/dozer-app/src/git_log.rs` → `crates/dozer-app/src/extensions/git_log.rs`
- Create: `crates/dozer-app/src/extensions.rs`
- Modify: `crates/dozer-app/src/main.rs`(`mod git_log;` → `mod extensions;`)
- Modify: `crates/dozer-app/src/workspace.rs`(`use crate::git_log;` → `use crate::extensions::git_log;`)

**Interfaces:**
- Produces: `crate::extensions::git_log::*` 在原 `crate::git_log::*` 的位置上原样可用(内容零改动,只挪了文件位置)。

- [x] **Step 1: 建目录、搬文件**

```bash
mkdir -p crates/dozer-app/src/extensions
git mv crates/dozer-app/src/git_log.rs crates/dozer-app/src/extensions/git_log.rs
```

- [x] **Step 2: 新建 `extensions.rs` 模块入口**

`crates/dozer-app/src/extensions.rs`:

```rust
//! 阶段 1 扩展化重构落地的模块目录:每个子模块拥有自己的 `Message`/
//! `State`/`update`/`view`,`workspace.rs` 内核只做包装转发(见
//! `docs/superpowers/specs/2026-08-07-git-log-extension-pilot-design.md`)。
//! 目前只有 `git_log` 一个试点;browser/todo 等面板视后续排期跟进。

pub mod git_log;
```

- [x] **Step 3: `main.rs` 模块声明**

把 `mod git_log;`(第 8 行)删掉,按字母序插入:

```rust
mod delivery;
mod extensions;
mod fonts;
```

(`extensions` 排在 `delivery` 之后、`fonts` 之前,与文件里其余 `mod` 声明的字母序一致。)

- [x] **Step 4: `workspace.rs` 的 import**

第 36 行:

```rust
use crate::git_log;
```

改成:

```rust
use crate::extensions::git_log;
```

(`workspace.rs` 其余所有 `git_log::xxx` 调用点不用动——本地别名名字没变,只是它现在指向
`extensions::git_log` 而不是顶层 `git_log`。)

- [x] **Step 5: 编译 + 测试确认纯移动没有破坏任何东西**

Run: `cargo build -p dozer-app && cargo test -p dozer-app git_log::`
Expected: 编译通过;`extensions::git_log` 模块里原有的 6 组测试全部 PASS(测试内容/路径没有变化,只是现在挂在新模块路径下)。

- [x] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions.rs crates/dozer-app/src/extensions/git_log.rs \
        crates/dozer-app/src/main.rs crates/dozer-app/src/workspace.rs
git commit -m "refactor(dozer-app): move git_log.rs into extensions/ module directory"
```

---

### Task 2: `git_log` 模块自己的 `Message`/`State`/`update`/`request_refresh`

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs`

**Interfaces:**
- Produces:
  - `pub enum Message { SelectCommit(git2::Oid), LoadMore, DetailLoaded(PathBuf, git2::Oid, Result<CommitDetail, String>), SnapshotLoaded(PathBuf, usize, Result<GitLogSnapshot, String>) }`(`Debug, Clone`)
  - `pub struct State { .. }`(`Default`,6 个私有字段,对应现在 `App` 上的 6 个 `git_log_*`)
  - `impl State`:`next_load_more_count(&self) -> usize`、`cache_max_count(&self) -> usize`、
    `cache_repo_path(&self) -> Option<&Path>`、`selected(&self) -> Option<git2::Oid>`、
    `set_restore_after_load(&mut self, oid: Option<git2::Oid>)`
  - `pub fn update(state: &mut State, msg: Message, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static) -> Option<Message>`
  - `pub fn request_refresh(state: &mut State, repo_path: PathBuf, max_count: usize, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`
  - `pub fn view(state: &State) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>`(签名变了,内部渲染逻辑不变)
- Consumes(下个任务用):以上全部 pub 项。Task 3 的内核代码会调用这些。

- [x] **Step 1: 删除对顶层 `Message` 的依赖,定义自己的 `Message`**

删除文件顶部这一行:

```rust
use crate::workspace::Message;
```

在 `GitLogSnapshot`/`impl GitLogSnapshot` 之后(大约现有第 105 行,`commit_detail` 函数之前
的合适位置——紧跟在类型定义区,`default_settings`/`build` 函数之前)插入:

```rust
/// Git Log 模块自己的消息类型——内核(`workspace.rs`)只认一个包装变体
/// `Message::GitLog(extensions::git_log::Message)`,这个模块本身不 import
/// 顶层 `Message`,不知道自己被包在哪个外层类型里。
#[derive(Debug, Clone)]
pub enum Message {
    SelectCommit(git2::Oid),
    LoadMore,
    DetailLoaded(PathBuf, git2::Oid, Result<CommitDetail, String>),
    SnapshotLoaded(PathBuf, usize, Result<GitLogSnapshot, String>),
}
```

（这个位置在文件里 `CommitDetail` 定义**之后**才合法,因为 `DetailLoaded` 用到它——写代码时
确认插入点在 `pub struct CommitDetail { .. }` 之后,`commit_detail` 函数之后即可,不用卡死在
"紧跟 `GitLogSnapshot`"这个粗略描述上。）

- [x] **Step 2: 定义 `State`**

紧跟 `Message` 定义之后:

```rust
/// Git Log 面板的全部状态。现在挂在 `App`(不按项目分,见设计文档"非
/// 目标"——这次纯重构不改这个现状),以后要改成按项目分的话,类型本身
/// 不用变,只是挪个持有位置。
#[derive(Default)]
pub struct State {
    cache: Option<GitLogSnapshot>,
    error: Option<String>,
    selected: Option<git2::Oid>,
    detail: Option<Result<CommitDetail, String>>,
    /// 最近一次派发的 `build` 请求 (repo_path, max_count)——落地时核对
    /// 还对不对得上"现在真正需要的",不对就丢弃。
    pending: Option<(PathBuf, usize)>,
    /// "加载更多"发起前记下的选中提交,新快照落地后据此还原选中态。
    restore_after_load: Option<git2::Oid>,
}

impl State {
    /// "加载更多"按钮下一个请求的 `max_count`:有缓存则在当前基础上
    /// `+ LOAD_MORE_STEP`,否则回落 `DEFAULT_MAX_COMMITS`。
    pub fn next_load_more_count(&self) -> usize {
        self.cache
            .as_ref()
            .map(|c| c.max_count() + LOAD_MORE_STEP)
            .unwrap_or(DEFAULT_MAX_COMMITS)
    }

    /// 当前缓存的 `max_count`(无缓存则回落 `DEFAULT_MAX_COMMITS`)——用于
    /// "内容不变、只是要重新拉一遍"的场景(`.git` 引用变化触发的重建),
    /// 跟"加载更多"要的 `next_load_more_count()`(会 `+LOAD_MORE_STEP`)
    /// 是两回事,内核代码里不要混用。
    pub fn cache_max_count(&self) -> usize {
        self.cache
            .as_ref()
            .map(|c| c.max_count())
            .unwrap_or(DEFAULT_MAX_COMMITS)
    }

    /// 当前缓存快照所属的仓库路径(`None` = 还没有缓存)。内核靠它判断
    /// "缓存是不是已经属于当前聚焦项目",不用时不重建。
    pub fn cache_repo_path(&self) -> Option<&Path> {
        self.cache.as_ref().map(|c| c.repo_path())
    }

    /// 当前选中的提交(`None` = 未选中)。内核发起"加载更多"前需要先读一次
    /// 这个值——`request_refresh` 会把它清空,内核得自己先存一份,请求
    /// 落地后再用 `set_restore_after_load` 传回来。
    pub fn selected(&self) -> Option<git2::Oid> {
        self.selected
    }

    /// `request_refresh` 落地新快照之前,内核用这个把"发起刷新前选中的
    /// 提交"记下来,新快照真正落地(`SnapshotLoaded` 处理完)时
    /// `update()` 会据此还原选中态(见其返回值 `Some(Message::SelectCommit)`
    /// 那条路径)。
    pub fn set_restore_after_load(&mut self, oid: Option<git2::Oid>) {
        self.restore_after_load = oid;
    }
}
```

`Path` 需要在文件顶部 `use std::path::{Path, PathBuf};` 已有(现有代码本就 `use` 了两者,不用
新增 import)。

- [x] **Step 3: `update` 函数**

紧跟 `State`/`impl State` 之后:

```rust
/// 处理 `SelectCommit`/`DetailLoaded`/`SnapshotLoaded` 三种消息。
/// `LoadMore` 需要内核才知道的"当前聚焦项目路径",不在这里处理——传进来
/// 会直接 panic,调用方(`workspace.rs`)必须在转发前先拦掉这一种(见
/// 设计文档"内核转发不是无差别盲转"）。
///
/// 返回值:`Some(next)` = 这次处理还产生了一条要递归分发的后续消息(目前
/// 只有 `SnapshotLoaded` 落地后恢复选中提交这一种情况)。
pub fn update(
    state: &mut State,
    msg: Message,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
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
                let result =
                    tokio::task::spawn_blocking(move || commit_detail(&repo_path2, oid))
                        .await
                        .unwrap_or_else(|e| Err(format!("详情加载任务失败: {e}")));
                emit(Message::DetailLoaded(repo_path, oid, result));
            });
            None
        }
        Message::DetailLoaded(repo_path, oid, result) => {
            let still_current = state.cache.as_ref().map(|c| c.repo_path())
                == Some(repo_path.as_path())
                && state.selected == Some(oid);
            if still_current {
                state.detail = Some(result);
            }
            // 否则:项目已切换,或用户点了别的提交——这份结果过期了,丢弃。
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
            unreachable!(
                "LoadMore 由内核在 Message::GitLog 分支里直接处理(需要仓库路径),不会转发到这里"
            )
        }
    }
}
```

- [x] **Step 4: `request_refresh` 函数**

紧跟 `update` 之后:

```rust
/// 异步重建 Git Log 快照,`max_count` 由调用方决定(打开面板/引用变化用
/// `DEFAULT_MAX_COMMITS`,"加载更多"用 `State::next_load_more_count()`)。
/// 现有 `App::spawn_git_log_refresh` 的搬家版本,行为不变。
pub fn request_refresh(
    state: &mut State,
    repo_path: PathBuf,
    max_count: usize,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    state.selected = None;
    state.detail = None;
    state.restore_after_load = None;
    state.pending = Some((repo_path.clone(), max_count));
    handle.spawn(async move {
        let repo_path2 = repo_path.clone();
        let result = tokio::task::spawn_blocking(move || build(&repo_path2, max_count))
            .await
            .unwrap_or_else(|e| Err(format!("Git Log 加载任务失败: {e}")));
        emit(Message::SnapshotLoaded(repo_path, max_count, result));
    });
}
```

- [x] **Step 5: 改 `view`/`GitLogCanvas`/`load_more` 按钮吃本模块 `Message`**

`GitLogCanvas` 的 `impl canvas::Program<Message, iced_widget::Theme, iced_widget::Renderer>`
不用改签名文字(`Message` 现在解析到本模块刚定义的类型,不再是 `crate::workspace::Message`);
只改函数体里这一行(原 `update` 方法内):

```rust
        Some(canvas::Action::publish(Message::GitLogSelectCommit(row.oid)).and_capture())
```

改成:

```rust
        Some(canvas::Action::publish(Message::SelectCommit(row.oid)).and_capture())
```

`view` 函数签名:

```rust
pub fn view<'a>(
    snapshot: Option<&'a GitLogSnapshot>,
    error: Option<&'a str>,
    selected: Option<git2::Oid>,
    detail: Option<&'a Result<CommitDetail, String>>,
    loading: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
```

改成新签名 + 开头几行局部绑定,让函数体主干(`if snapshot.rows.is_empty() { ... }` 往后)
原样保留、不用逐处替换 `snapshot`/`selected`/`detail`:

```rust
pub fn view(state: &State) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let error = state.error.as_deref();
    if let Some(err) = error {
        return container(text(format!("git log 读取失败: {err}")).color(theme::RED))
            .padding(16)
            .into();
    }
    let loading = state.pending.is_some();
    let Some(snapshot) = state.cache.as_ref() else {
        let text_content = if loading {
            "加载中…"
        } else {
            "未打开项目"
        };
        return container(text(text_content).color(theme::DIM))
            .padding(16)
            .into();
    };
    let selected = state.selected;
    let detail = state.detail.as_ref();
    // ↓ 原函数体从这里(`if snapshot.rows.is_empty() { ... }`)开始原样保留 ↓
```

再把 `load_more` 按钮那一行:

```rust
    .on_press_maybe((!loading).then_some(Message::GitLogLoadMore))
```

改成:

```rust
    .on_press_maybe((!loading).then_some(Message::LoadMore))
```

- [x] **Step 6: 编译确认模块自洽**

Run: `cargo build -p dozer-app 2>&1 | head -100`
Expected: `extensions/git_log.rs` 自身不再报"`Message` 未定义"之类的错;此时 `workspace.rs`
那边大概率还报错(还在用旧的 `GitLogSelectCommit` 等顶层变体、旧的 `git_log::view(...)` 五参数
调用),这些留给 Task 3 修——本步骤只要求 `extensions/git_log.rs` **这个文件本身**没有编译
错误(可以用 `cargo check -p dozer-app 2>&1 | grep "extensions/git_log.rs"` 单独确认这个文件
没有报错,即使整个 crate 因为 workspace.rs 还没跟进而编译不过)。

- [x] **Step 7: 新增 `update`/`request_refresh`/`State` 的单测**

在文件末尾 `mod tests` 里追加(`use super::*;` 已存在,直接加测试函数):

```rust
    fn snapshot_at(repo_path: &Path, max_count: usize) -> GitLogSnapshot {
        GitLogSnapshot {
            repo_path: repo_path.to_path_buf(),
            rows: Vec::new(),
            max_column: 0,
            head_branch: None,
            max_count,
        }
    }

    #[tokio::test]
    async fn select_commit_sets_selected_and_clears_detail() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            detail: Some(Ok(CommitDetail { files: Vec::new() })),
            ..State::default()
        };
        let oid = git2::Oid::from_bytes(&[1; 20]).unwrap();
        let handle = tokio::runtime::Handle::current();
        let result = update(&mut state, Message::SelectCommit(oid), &handle, |_| {});
        assert_eq!(state.selected, Some(oid));
        assert!(state.detail.is_none());
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn select_commit_without_cache_sets_selected_but_spawns_nothing() {
        let mut state = State::default();
        let oid = git2::Oid::from_bytes(&[2; 20]).unwrap();
        let handle = tokio::runtime::Handle::current();
        let result = update(&mut state, Message::SelectCommit(oid), &handle, |_| {
            panic!("无缓存时不该 emit 任何消息");
        });
        assert_eq!(state.selected, Some(oid));
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detail_loaded_writes_when_repo_path_and_selected_match() {
        let repo_path = PathBuf::from("/tmp/repo");
        let oid = git2::Oid::from_bytes(&[3; 20]).unwrap();
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            selected: Some(oid),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let result = update(
            &mut state,
            Message::DetailLoaded(repo_path, oid, Ok(CommitDetail { files: Vec::new() })),
            &handle,
            |_| {},
        );
        match &state.detail {
            Some(Ok(detail)) => assert!(detail.files.is_empty()),
            other => panic!("期望 Some(Ok(空 CommitDetail)),实际 {other:?}"),
        }
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detail_loaded_discarded_when_repo_path_mismatches() {
        let mut state = State {
            cache: Some(snapshot_at(Path::new("/tmp/a"), 10)),
            selected: Some(git2::Oid::from_bytes(&[4; 20]).unwrap()),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DetailLoaded(
                PathBuf::from("/tmp/b"),
                git2::Oid::from_bytes(&[4; 20]).unwrap(),
                Ok(CommitDetail { files: Vec::new() }),
            ),
            &handle,
            |_| {},
        );
        assert!(state.detail.is_none(), "仓库路径对不上,结果应被丢弃");
    }

    #[tokio::test]
    async fn detail_loaded_discarded_when_selected_mismatches() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            selected: Some(git2::Oid::from_bytes(&[5; 20]).unwrap()),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DetailLoaded(
                repo_path,
                git2::Oid::from_bytes(&[6; 20]).unwrap(),
                Ok(CommitDetail { files: Vec::new() }),
            ),
            &handle,
            |_| {},
        );
        assert!(state.detail.is_none(), "选中的提交对不上,结果应被丢弃");
    }

    #[tokio::test]
    async fn snapshot_loaded_lands_when_pending_matches() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            pending: Some((repo_path.clone(), 10)),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let result = update(
            &mut state,
            Message::SnapshotLoaded(repo_path.clone(), 10, Ok(snapshot_at(&repo_path, 10))),
            &handle,
            |_| {},
        );
        assert!(state.cache.is_some());
        assert!(state.error.is_none());
        assert!(state.pending.is_none());
        assert!(result.is_none(), "restore_after_load 为 None 时不该产生后续消息");
    }

    #[tokio::test]
    async fn snapshot_loaded_discarded_when_pending_mismatches() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            pending: Some((repo_path.clone(), 10)),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::SnapshotLoaded(repo_path.clone(), 20, Ok(snapshot_at(&repo_path, 20))),
            &handle,
            |_| {},
        );
        assert!(state.cache.is_none(), "max_count 对不上,不该落地");
        assert_eq!(state.pending, Some((repo_path, 10)), "pending 也不该被清掉");
    }

    #[tokio::test]
    async fn snapshot_loaded_error_clears_cache_and_sets_error() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            pending: Some((repo_path.clone(), 10)),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::SnapshotLoaded(repo_path, 10, Err("boom".to_string())),
            &handle,
            |_| {},
        );
        assert!(state.cache.is_none());
        assert_eq!(state.error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn snapshot_loaded_returns_select_commit_when_restore_after_load_set() {
        let repo_path = PathBuf::from("/tmp/repo");
        let oid = git2::Oid::from_bytes(&[7; 20]).unwrap();
        let mut state = State {
            pending: Some((repo_path.clone(), 10)),
            restore_after_load: Some(oid),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let result = update(
            &mut state,
            Message::SnapshotLoaded(repo_path.clone(), 10, Ok(snapshot_at(&repo_path, 10))),
            &handle,
            |_| {},
        );
        match result {
            Some(Message::SelectCommit(got)) => assert_eq!(got, oid),
            other => panic!("期望 Some(SelectCommit(oid)),实际 {other:?}"),
        }
        assert!(state.restore_after_load.is_none(), "取用后应清空");
    }

    #[test]
    fn next_load_more_count_with_cache_adds_step() {
        let state = State {
            cache: Some(snapshot_at(Path::new("/tmp/repo"), 200)),
            ..State::default()
        };
        assert_eq!(state.next_load_more_count(), 200 + LOAD_MORE_STEP);
    }

    #[test]
    fn next_load_more_count_without_cache_falls_back_to_default() {
        let state = State::default();
        assert_eq!(state.next_load_more_count(), DEFAULT_MAX_COMMITS);
    }

    #[test]
    fn cache_max_count_reflects_current_cache_without_adding_step() {
        let with_cache = State {
            cache: Some(snapshot_at(Path::new("/tmp/repo"), 37)),
            ..State::default()
        };
        assert_eq!(with_cache.cache_max_count(), 37, "不该像 next_load_more_count 那样 +LOAD_MORE_STEP");
        assert_eq!(State::default().cache_max_count(), DEFAULT_MAX_COMMITS);
    }

    #[test]
    fn cache_repo_path_reflects_cache_presence() {
        let repo_path = PathBuf::from("/tmp/repo");
        let with_cache = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            ..State::default()
        };
        assert_eq!(with_cache.cache_repo_path(), Some(repo_path.as_path()));
        assert_eq!(State::default().cache_repo_path(), None);
    }

    #[test]
    fn selected_and_set_restore_after_load_roundtrip() {
        let mut state = State::default();
        assert_eq!(state.selected(), None);
        let oid = git2::Oid::from_bytes(&[9; 20]).unwrap();
        state.selected = Some(oid);
        assert_eq!(state.selected(), Some(oid));
        state.set_restore_after_load(Some(oid));
        assert_eq!(state.restore_after_load, Some(oid));
        state.set_restore_after_load(None);
        assert_eq!(state.restore_after_load, None);
    }

    #[tokio::test]
    async fn request_refresh_resets_selection_and_records_pending() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            selected: Some(git2::Oid::from_bytes(&[8; 20]).unwrap()),
            detail: Some(Ok(CommitDetail { files: Vec::new() })),
            restore_after_load: Some(git2::Oid::from_bytes(&[8; 20]).unwrap()),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        request_refresh(&mut state, repo_path.clone(), 50, &handle, |_| {});
        assert!(state.selected.is_none());
        assert!(state.detail.is_none());
        assert!(state.restore_after_load.is_none());
        assert_eq!(state.pending, Some((repo_path, 50)));
    }
```

- [x] **Step 8: 运行新测试确认通过**

Run: `cargo test -p dozer-app --lib extensions::git_log:: 2>&1 | tail -80`
Expected: 新增 16 个测试(加上 Task 1 保留的 6 个,共 22 个)全部 PASS。若 `#[tokio::test]`
报缺 feature,检查根 `Cargo.toml` 的 `tokio = { version = "1", features = ["full"] }`——
`full` 已包含 `macros`/`rt`,不需要改 `Cargo.toml`。

- [x] **Step 9: Commit**

```bash
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "refactor(dozer-app): give git_log its own Message/State/update/view"
```

---

### Task 3: 内核接线——`workspace.rs` 改用包装转发

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `extensions::git_log::{Message, State, update, request_refresh, view}`(Task 2)

- [x] **Step 1: `App` 结构体字段合并**

把这 6 行(结构体定义处,约第 1457-1480 行区域):

```rust
    git_log_cache: Option<git_log::GitLogSnapshot>,
    git_log_error: Option<String>,
    git_log_selected: Option<git2::Oid>,
    git_log_detail: Option<Result<git_log::CommitDetail, String>>,
    git_log_pending: Option<(PathBuf, usize)>,
    git_log_restore_after_load: Option<git2::Oid>,
```

(连同各自的文档注释一起删除)替换成:

```rust
    /// Git Log 面板状态——自己的 `Message`/`update`/`view`,见
    /// `extensions::git_log`。`App` 级共享、不按项目分(现状,纯重构不改,
    /// 见 `sync_git_log_to_active_project`)。
    git_log: git_log::State,
```

- [x] **Step 2: `App::bootstrap` 初始化**

第 2991-2996 行(6 个 `git_log_*: None,`)替换成:

```rust
            git_log: git_log::State::default(),
```

- [x] **Step 3: 顶层 `Message` 枚举**

把这四个变体(约第 1146-1157 行,连同文档注释):

```rust
    GitLogSelectCommit(git2::Oid),
    GitLogDetailLoaded(PathBuf, git2::Oid, Result<git_log::CommitDetail, String>),
    GitLogLoadMore,
    GitLogSnapshotLoaded(PathBuf, usize, Result<git_log::GitLogSnapshot, String>),
```

替换成一个:

```rust
    /// Git Log 面板的全部消息,内核只转发不解读——见
    /// `extensions::git_log::Message`。
    GitLog(git_log::Message),
```

- [x] **Step 4: `update()` 里四支旧分支合并成两支**

删除这四个 match 分支(`Message::GitLogSelectCommit` / `Message::GitLogDetailLoaded` /
`Message::GitLogSnapshotLoaded` / `Message::GitLogLoadMore`,约第 4706-4783 行,内容见前面
勘探时读到的原文,整段删掉),换成:

```rust
            Message::GitLog(git_log::Message::LoadMore) => {
                let Some(path) = self
                    .active_workspace()
                    .and_then(|ws| ws.active_project_path())
                else {
                    return;
                };
                let next = self.git_log.next_load_more_count();
                // `request_refresh` 内部会把 `selected` 清空,所以必须在调用它之前
                // 先读出来,落地新快照后(`update()` 处理 `SnapshotLoaded` 那支)才能
                // 据此还原选中态——镜像现有 `Message::GitLogLoadMore` 分支"先记
                // selected,刷新,再把 restore_after_load 设回去"的顺序。
                let selected = self.git_log.selected();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::GitLog(m));
                };
                git_log::request_refresh(&mut self.git_log, path, next, &handle, emit);
                self.git_log.set_restore_after_load(selected);
            }
            Message::GitLog(msg) => {
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::GitLog(m));
                };
                if let Some(next) = git_log::update(&mut self.git_log, msg, &handle, emit) {
                    self.update(Message::GitLog(next));
                }
            }
```

- [x] **Step 5: `sync_git_log_to_active_project` 改用新入口**

```rust
    fn sync_git_log_to_active_project(&mut self) {
        let path = self
            .active_workspace()
            .and_then(|ws| ws.active_project_path());
        match path {
            Some(p) if self.git_log.cache_repo_path() != Some(p.as_path()) => {
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::GitLog(m));
                };
                git_log::request_refresh(
                    &mut self.git_log,
                    p,
                    git_log::DEFAULT_MAX_COMMITS,
                    &handle,
                    emit,
                );
            }
            None => {
                self.git_log = git_log::State::default();
            }
            _ => {}
        }
    }
```

（用到的 `cache_repo_path()` 访问器 Task 2 Step 2 已经加进 `impl State` 了,这里直接用。）

- [x] **Step 6: `ProjectFsChanged` 分支改用新访问器**

原代码里这一段(约第 4686-4703 行):

```rust
                if relevance == git_watch::Relevance::GitRefs
                    && self.active_project_id == Some(project_id)
                    && let Some(repo_path) = self
                        .git_log_cache
                        .as_ref()
                        .map(|c| c.repo_path().to_path_buf())
                    && self
                        .active_workspace()
                        .and_then(|ws| ws.active_project_path())
                        .as_deref()
                        == Some(repo_path.as_path())
                {
                    let max = self
                        .git_log_cache
                        .as_ref()
                        .map(|c| c.max_count())
                        .unwrap_or(git_log::DEFAULT_MAX_COMMITS);
                    self.spawn_git_log_refresh(&repo_path, max);
                }
```

改成:

```rust
                if relevance == git_watch::Relevance::GitRefs
                    && self.active_project_id == Some(project_id)
                    && let Some(repo_path) = self.git_log.cache_repo_path().map(|p| p.to_path_buf())
                    && self
                        .active_workspace()
                        .and_then(|ws| ws.active_project_path())
                        .as_deref()
                        == Some(repo_path.as_path())
                {
                    // 引用变化只是要"内容不变、重新拉一遍",窗口大小维持原样——
                    // 用 `cache_max_count()`(读当前缓存的 max_count),不是"加载
                    // 更多"专用、会 `+LOAD_MORE_STEP` 的 `next_load_more_count()`。
                    let max = self.git_log.cache_max_count();
                    let handle = self.handle.clone();
                    let proxy = self.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::GitLog(m));
                    };
                    git_log::request_refresh(&mut self.git_log, repo_path, max, &handle, emit);
                }
```

- [x] **Step 7: `LeftIconSelect`/`ProjectTabSwitch` 里的 `sync_git_log_to_active_project()` 调用点**

这两处(约第 4149-4151 行、第 4562-4564 行)调用方式不变,`sync_git_log_to_active_project`
本身签名没变,不用动调用点代码。

- [x] **Step 8: `App::view()` 的渲染调用**

约第 7048-7054 行:

```rust
        LeftView::GitLog => crate::git_log::view(
            app.git_log_cache.as_ref(),
            app.git_log_error.as_deref(),
            app.git_log_selected,
            app.git_log_detail.as_ref(),
            app.git_log_pending.is_some(),
        ),
```

改成:

```rust
        LeftView::GitLog => git_log::view(&app.git_log).map(Message::GitLog),
```

- [x] **Step 9: 编译,逐条修正**

Run: `cargo build -p dozer-app 2>&1 | head -150`
Expected: `State` 上用到的 `next_load_more_count`/`cache_max_count`/`cache_repo_path`/
`selected`/`set_restore_after_load` 五个访问器 Task 2 已经全部加好,这里应该只是些细节
类型错误(比如某处 `&self.git_log` vs `&mut self.git_log` borrow 冲突)。逐条修正直到
`cargo build -p dozer-app` 干净通过。**不要**为了让它编译过而改变行为语义(比如拿
`next_load_more_count` 顶替 `cache_max_count`)——两者语义不同,前面已经写明区别。

- [x] **Step 10: 全量测试 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt --check`
Expected: 全绿。

- [x] **Step 11: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/extensions/git_log.rs
git commit -m "refactor(dozer-app): route Git Log messages through extensions::git_log"
```

---

### Task 4: 全量校验与人工验收

**Files:** 无新增/修改(纯校验任务)

- [x] **Step 1: 全 workspace 构建 + 测试 + clippy + fmt**

Run: `cargo build && cargo test && cargo clippy --all-targets && cargo fmt --check`
Expected: 全部 crate 编译通过、测试全绿、无 clippy 警告、无格式差异。

- [ ] **Step 2: 人工验收(`cargo run -p dozer-app`)**

打开一个 git 仓库项目:
1. 点左图标栏进入 Git Log 面板——提交图应正常画出来(track 连线、圆点、分支标签、HEAD 箭头)。
2. 点一行提交——右侧应弹出详情面板(文件列表 + diff 文本)。
3. 点"加载更多提交 (+200)"——图应变长,且**之前选中的提交详情应该还在**(这是
   `restore_after_load` 那条级联逻辑要保证的行为)。
4. 切到另一个项目页签(如果该项目也是 git 仓库且也打开过 Git Log 面板)——提交图应该跟着切
   过去;如果该项目还没打开过 Git Log 面板,行为应该跟改造前一致(不报错、不崩溃,大概率是
   显示"加载中…"然后正常拉出新项目的提交图)。
5. 在仓库里手动切一个分支(命令行 `git checkout -b test-branch`)——面板应该(通过
   `git_watch` 探测到 `.git` 引用变化)自动重建提交图,`max_count` 保持不变(不应该意外变成
   `next_load_more_count` 那种 `+200` 的值)。
6. 打开一个不是 git 仓库的项目——面板应显示"未打开项目"或等价的空态文案,不报错。

若上述任一步与预期不符,对照 Task 2/Task 3 的具体分支重新核对(尤其"加载更多"的
`restore_after_load` 传递路径,和"`.git` 引用变化"用的是 `cache_max_count()` 而不是
`next_load_more_count()` 这两个最容易犯的语义混淆点)。

- [x] **Step 3: 确认没有遗留未提交的改动**

Run: `git status`
Expected: 干净(所有改动都已在前面各 Task 的 Step 里提交)。
