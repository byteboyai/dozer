# 验收(Acceptance)面板 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把验收功能从"`PreviewPane` 里靠终端横幅点开的特殊 tab"提升成右侧 rail 图标可选的独立面板 `RightView::Acceptance`,拆成自洽模块 `extensions::acceptance`(自己的 `Message`/`WorkspaceState`/`update`/`view`),面板自带 unified diff 精简预览,不再依赖/耦合 `PreviewPane`。

**Architecture:** 新文件 `crates/dozer-app/src/extensions/acceptance.rs`。新增内核级公用函数 `delivery::file_diff`(单文件 unified diff)。`preview.rs` 删除 `TabKind::Acceptance` 相关的一切,回归纯文件/网页 tab。`workspace.rs` 新增 `RightView::Acceptance`/`RailButton::RightAcceptance`,`right_icon_rail` 按当前激活 tab 的 `delivery_pending` 画徽标。

**Tech Stack:** Rust workspace;iced 0.14;`git2`(已有依赖,新增 `diff_tree_to_workdir_with_index` 调用);`tokio::runtime::Handle` + `emit: impl Fn(Message) + Send + 'static` 回调风格(同前五个试点)。

## Global Constraints

- 这次是真正的 UX 改动,不是纯重构——横幅删除、rail 新图标、点变更文件从"跳左侧预览"
  变成"原地展开 diff"都是有意的行为变化,已经过 brainstorming 确认;除此之外(验收
  业务逻辑本身、单会话语义)一律不变。
- `extensions::acceptance` 不得依赖 `crate::preview` 的任何类型/消息(`TabKind`/
  `Message::PreviewOpenPath` 等)——用户明确要求的架构原则:extension 之间只能通过
  消息通信,不能耦合。
- `Open`/`Reject` 两条消息内核拦截,不进 `acceptance::update`(会 `unreachable!`)。
- `Loaded`/`DiffLoaded`/`Done` 三条异步结果消息按自带的 `project_id` 走
  `with_project`,不能走 `with_focused_project`。
- `delivery::file_diff` 与 `git_log::commit_detail` 各自独立实现,不共用代码/常量。
- 每个任务结束都要 `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过。
- 设计文档:`docs/superpowers/specs/2026-08-08-acceptance-pane-design.md`(有疑问以它为准)。

---

### Task 1: `delivery::file_diff`——单文件 unified diff

**Files:**
- Modify: `crates/dozer-app/src/delivery.rs`

**Interfaces:**
- Produces:`pub fn file_diff(repo_path: &Path, path: &str) -> Result<String, String>`。

- [ ] **Step 1: 写失败的测试**

`delivery.rs` 现有 `#[cfg(test)] mod tests`(357 行起)已经有一个建临时 git 仓库的
辅助函数 `fn mkrepo() -> (tempfile::TempDir, std::path::PathBuf)`(359 行):建仓库、
写一个 `a.txt`(内容 `"one\n"`)、`git add . && git commit`,返回 `(TempDir 句柄,
仓库路径)`。直接复用这个函数,不要新写一套:

```rust
#[test]
fn file_diff_new_file_shows_all_added_lines() {
    let (dir, repo) = mkrepo();
    std::fs::write(dir.path().join("new.txt"), "a\nb\n").unwrap();
    let diff = file_diff(&repo, "new.txt").unwrap();
    assert!(diff.contains("+a"));
    assert!(diff.contains("+b"));
}

#[test]
fn file_diff_modified_file_shows_plus_minus_lines() {
    let (dir, repo) = mkrepo();
    // `mkrepo()` 已经提交了内容为 "one\n" 的 a.txt,替换成不同内容,
    // 应该同时看到删除旧行(-)和新增行(+)。
    std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
    let diff = file_diff(&repo, "a.txt").unwrap();
    assert!(diff.contains("-one"));
    assert!(diff.contains("+two"));
}

#[test]
fn file_diff_truncates_when_too_long() {
    let (dir, repo) = mkrepo();
    let big: String = (0..5000).map(|i| format!("line{i}\n")).collect();
    std::fs::write(dir.path().join("big.txt"), big).unwrap();
    let diff = file_diff(&repo, "big.txt").unwrap();
    assert!(diff.contains("已截断显示"));
    assert!(diff.len() < 21_000, "截断后不应远超 FILE_DIFF_MAX_CHARS");
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p dozer-app file_diff -- --nocapture
```

Expected: 编译失败(`file_diff` 未定义)。

- [ ] **Step 3: 实现 `file_diff`**

在 `delivery.rs` 里(放在 `pub fn changes` 之后,同属"读 git 状态"这一组函数):

```rust
/// 单文件相对验收基线(有上次沉淀取那个 ref,没有则 HEAD——与
/// `changes()` 用的同一套 base 解析)的 unified diff 原始文本。用
/// `diff_tree_to_workdir_with_index`(基线 tree vs 当前工作区,含已 stage
/// 的改动 + 未跟踪文件)。过长截断,截断阈值/提示文案与
/// `git_log::commit_detail` 的 `MAX_PATCH_CHARS` 处理方式类似但独立实现
/// (两个模块不共用代码,见设计文档"关键语义确认")。
const FILE_DIFF_MAX_CHARS: usize = 20_000;

pub fn file_diff(repo_path: &Path, path: &str) -> Result<String, String> {
    let repo = git2::Repository::open(repo_path).map_err(|e| e.message().to_string())?;
    let base_ref = last_accepted(repo_path)
        .map(|(n, _)| format!("{ACCEPTED_REF_PREFIX}{n}"))
        .or_else(|| head_commit(repo_path).map(|_| "HEAD".to_string()));
    let old_tree = match base_ref {
        Some(refname) => {
            let obj = repo
                .revparse_single(&refname)
                .map_err(|e| e.message().to_string())?;
            Some(obj.peel_to_tree().map_err(|e| e.message().to_string())?)
        }
        None => None,
    };
    let mut opts = git2::DiffOptions::new();
    opts.pathspec(path);
    opts.include_untracked(true);
    opts.recurse_untracked_dirs(true);
    let diff = repo
        .diff_tree_to_workdir_with_index(old_tree.as_ref(), Some(&mut opts))
        .map_err(|e| e.message().to_string())?;

    let mut patch = String::new();
    let mut truncated = false;
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        if truncated {
            return true;
        }
        if patch.len() >= FILE_DIFF_MAX_CHARS {
            truncated = true;
            patch.push_str("\n… diff 过长,已截断显示\n");
            return true;
        }
        let prefix = match line.origin() {
            '+' | '-' | ' ' => line.origin().to_string(),
            _ => String::new(),
        };
        patch.push_str(&prefix);
        patch.push_str(&String::from_utf8_lossy(line.content()));
        true
    })
    .map_err(|e| e.message().to_string())?;

    if patch.is_empty() {
        return Err(format!("{path}: 没有可显示的改动"));
    }
    Ok(patch)
}
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p dozer-app file_diff
```

Expected: 三个新测试 PASS。

- [ ] **Step 5: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/delivery.rs
git commit -m "feat(dozer-app): add delivery::file_diff for single-file unified diff"
```

---

### Task 2: 新图标 `IconKind::BadgeCheck`

**Files:**
- Create: `crates/dozer-app/assets/icons/badge-check.svg`
- Modify: `crates/dozer-app/src/icons.rs`

**Interfaces:**
- Produces:`icons::IconKind::BadgeCheck` 变体,可传给现有 `icons::view(kind, size,
  color)` 使用。

- [ ] **Step 1: 加 SVG 资源**

创建 `crates/dozer-app/assets/icons/badge-check.svg`(Lucide `badge-check`,与仓库
现有图标同款 24×24 `currentColor` 描边格式):

```svg
<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
>
  <path d="M3.85 8.62a4 4 0 0 1 4.78-4.77 4 4 0 0 1 6.74 0 4 4 0 0 1 4.78 4.78 4 4 0 0 1 0 6.74 4 4 0 0 1-4.77 4.78 4 4 0 0 1-6.75 0 4 4 0 0 1-4.78-4.77 4 4 0 0 1 0-6.76Z" />
  <path d="m9 12 2 2 4-4" />
</svg>
```

- [ ] **Step 2: 加枚举变体**

`crates/dozer-app/src/icons.rs` 的 `IconKind` 枚举,在 `BarChart3` 之后插入:

```rust
/// Agent 用量统计面板的图标(Lucide bar-chart-3)。
BarChart3,
/// 验收面板 rail 图标(Lucide badge-check)。
BadgeCheck,
```

`IconKind::bytes()` 的 `match` 里对应加一行(按现有代码组织,插在 `BarChart3` 那行
之后):

```rust
IconKind::BarChart3 => include_bytes!("../assets/icons/bar-chart-3.svg"),
IconKind::BadgeCheck => include_bytes!("../assets/icons/badge-check.svg"),
```

- [ ] **Step 3: 编译确认没有漏改的 match 分支**

```bash
cargo build -p dozer-app
```

Expected: 编译通过(`IconKind` 上没有其它穷举 `match`,若报错则按报错位置补分支)。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/assets/icons/badge-check.svg crates/dozer-app/src/icons.rs
git commit -m "feat(dozer-app): add BadgeCheck icon for the acceptance pane"
```

---

### Task 3: `extensions::acceptance`——类型骨架 + `update` + 内核直调函数

**Files:**
- Create: `crates/dozer-app/src/extensions/acceptance.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(加 `pub mod acceptance;`)

**Interfaces:**
- Produces:`pub struct WorkspaceState`(`session()`/`set_session()`/
  `clear_session()`/`delivery_pending` 无关——那是 `SessionTab` 字段,不在这里)、
  `pub enum Message`、`pub fn update(..)`、`pub fn spawn_open(..)`。

- [ ] **Step 1: 类型定义**

创建 `crates/dozer-app/src/extensions/acceptance.rs`:

```rust
//! 验收(甲方验收)面板:项目目标标准复选、变更文件列表 + 自带的精简
//! unified diff 预览、验收意见、通过/打回。阶段 1 扩展化重构第六个试点,
//! 但跟前五个不同——这是一次产品级 UX 改动(从 `PreviewPane` 里靠横幅
//! 点开的特殊 tab 提升为独立 rail 面板),设计见
//! `docs/superpowers/specs/2026-08-08-acceptance-pane-design.md`。
use crate::delivery::FileChange;
use crate::goal::Goal;
use crate::workspace::AddrEvent;
use dozer_client::Client;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// 挂在每个 Workspace 上的验收面板状态。
#[derive(Default)]
pub struct WorkspaceState {
    session: Option<AcceptanceSession>,
}

impl WorkspaceState {
    pub fn session(&self) -> Option<&AcceptanceSession> {
        self.session.as_ref()
    }

    /// 供内核 `Open` 拦截处理后落地新会话——不经过 `Message::Loaded`,
    /// 因为 `Loaded` 本身就是通过 `update` 落地的(见 `update` 的
    /// `Loaded` 分支)。这个方法留给测试/未来直接构造场景用,`update`
    /// 内部处理 `Loaded` 时直接赋值字段,不调用它——两条路径做同一件事,
    /// 保留这个方法只是为了让"新建会话"这个操作有一个命名清楚的入口。
    #[cfg(test)]
    fn set_session_for_test(&mut self, session: AcceptanceSession) {
        self.session = Some(session);
    }

    /// 供内核 `Reject` 拦截处理完成后调用——清空验收会话状态,不经过
    /// `Message`/`update`(同 Todo 试点"内核完成终端相关部分后直调扩展
    /// 普通函数"的处理方式)。
    pub fn clear_session(&mut self) {
        self.session = None;
    }

    /// 供内核 `Reject` 拦截处理的"来源会话已结束"降级路径调用——不清空
    /// 当前会话,只在上面写错误文案,让用户看到(现有 `acceptance_reject`
    /// 的降级路径)。
    pub fn set_error(&mut self, e: String) {
        if let Some(session) = &mut self.session {
            session.error = Some(e);
        }
    }
}

/// 一次进行中的验收(现有 `AcceptanceView` 的搬家版本)。
pub struct AcceptanceSession {
    repo: PathBuf,
    source_tab_id: usize,
    goal: Option<Goal>,
    changes: Vec<FileChange>,
    checked: Vec<bool>,
    /// 当前展开了 diff 的文件下标(对应 `changes` 的下标)。
    expanded: HashSet<usize>,
    /// diff 懒加载缓存:未展开过或仍在加载中的文件不在这个 map 里。
    diffs: HashMap<usize, Result<String, String>>,
    comment: String,
    comment_editing: bool,
    error: Option<String>,
    accepted_version: Option<u32>,
}

/// 对应现有 8 个 `Acceptance*` 变体去前缀搬来,新增 `ToggleDiff`/
/// `DiffLoaded` 两条支撑 diff 手风琴。`Open`/`Reject` 内核拦截,不进
/// `update`(见下)。
#[derive(Debug, Clone)]
pub enum Message {
    Open(usize),
    Loaded(i64, PathBuf, usize, Option<Goal>, Vec<FileChange>),
    Toggle(usize),
    ToggleDiff(usize),
    DiffLoaded(i64, usize, Result<String, String>),
    CommentClick,
    CommentEvent(AddrEvent),
    Accept,
    Reject,
    Done(i64, Result<u32, String>),
}
```

- [ ] **Step 2: `update`**

紧接着加:

```rust
/// 处理 `Open`/`Reject` 之外的全部消息。内核在到达这里之前已经拦截了
/// 这两条(需要终端会话域能力),它们传进来会 `unreachable!`(同 Files
/// 试点 `CopyPath` 的处理方式)。
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::Open(..) => unreachable!("由内核拦截处理,见 Message::Open 文档"),
        Message::Loaded(_, repo, source_tab_id, goal, changes) => {
            let n = goal.as_ref().map(|g| g.criteria.len()).unwrap_or(0);
            ws_state.session = Some(AcceptanceSession {
                repo,
                source_tab_id,
                goal,
                changes,
                checked: vec![false; n],
                expanded: HashSet::new(),
                diffs: HashMap::new(),
                comment: String::new(),
                comment_editing: false,
                error: None,
                accepted_version: None,
            });
        }
        Message::Toggle(i) => {
            if let Some(session) = &mut ws_state.session
                && let Some(c) = session.checked.get_mut(i)
            {
                *c = !*c;
            }
        }
        Message::ToggleDiff(i) => {
            let Some(session) = &mut ws_state.session else {
                return;
            };
            if !session.expanded.insert(i) {
                // 已经展开过 → 这次是收起。
                session.expanded.remove(&i);
                return;
            }
            if session.diffs.contains_key(&i) {
                return; // 已有缓存,不重新加载。
            }
            let Some(fc) = session.changes.get(i) else {
                return;
            };
            let repo = session.repo.clone();
            let path = fc.path.clone();
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    crate::delivery::file_diff(&repo, &path)
                })
                .await
                .unwrap_or_else(|e| Err(format!("任务失败: {e}")));
                emit(Message::DiffLoaded(project_id, i, result));
            });
        }
        Message::DiffLoaded(_, i, result) => {
            if let Some(session) = &mut ws_state.session {
                session.diffs.insert(i, result);
            }
        }
        Message::CommentClick => {
            if let Some(session) = &mut ws_state.session {
                session.comment_editing = true;
            }
        }
        Message::CommentEvent(ev) => {
            if let Some(session) = &mut ws_state.session {
                match ev {
                    AddrEvent::Text(s) => session.comment.push_str(&s),
                    AddrEvent::Backspace => {
                        session.comment.pop();
                    }
                    AddrEvent::Submit | AddrEvent::Cancel => session.comment_editing = false,
                }
            }
        }
        Message::Accept => {
            let Some(session) = &mut ws_state.session else {
                return;
            };
            session.error = None;
            let repo = session.repo.clone();
            let goal_title = session
                .goal
                .as_ref()
                .map(|g| g.title.clone())
                .unwrap_or_default();
            let checked: Vec<String> = session
                .goal
                .as_ref()
                .map(|g| {
                    g.criteria
                        .iter()
                        .zip(&session.checked)
                        .filter(|(_, c)| **c)
                        .map(|(s, _)| s.clone())
                        .collect()
                })
                .unwrap_or_default();
            let comment = session.comment.clone();
            let client = client.clone();
            handle.spawn(async move {
                let repo2 = repo.clone();
                let accepted =
                    tokio::task::spawn_blocking(move || crate::delivery::accept(&repo2)).await;
                let result = match accepted {
                    Ok(Ok(n)) => {
                        let ref_name = format!("{}{n}", crate::delivery::ACCEPTED_REF_PREFIX);
                        let ts_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);
                        if let Err(e) = client
                            .record_acceptance(
                                &repo.to_string_lossy(),
                                &goal_title,
                                &checked,
                                "accepted",
                                &comment,
                                &ref_name,
                                ts_ms,
                            )
                            .await
                        {
                            Err(format!("已沉淀 v{n},但记录落库失败: {e}"))
                        } else {
                            Ok(n)
                        }
                    }
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(e) => Err(format!("任务失败: {e}")),
                };
                emit(Message::Done(project_id, result));
            });
        }
        Message::Reject => unreachable!("由内核拦截处理,见 Message::Reject 文档"),
        Message::Done(_, result) => {
            if let Some(session) = &mut ws_state.session {
                match result {
                    Ok(n) => session.accepted_version = Some(n),
                    Err(e) => session.error = Some(e),
                }
            }
        }
    }
}
```

- [ ] **Step 3: 内核直调的自由函数 `spawn_open`**

紧接着加:

```rust
/// 内核在 `Open` 拦截处理里调用(已经清空了 `SessionTab.delivery_pending`、
/// 算好了 `cwd`,这两步是终端会话域操作,不在这个函数里做)。异步读
/// `delivery::repo_root`/`goal::parse_goal`/`delivery::changes`,完成后
/// `emit(Loaded(..))`。现有 `Message::AcceptanceOpen` 处理器里
/// `io.handle.spawn` 那部分逻辑的搬家版本。
pub fn spawn_open(
    project_id: i64,
    tab_id: usize,
    cwd: PathBuf,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let loaded = tokio::task::spawn_blocking(move || {
            let repo = crate::delivery::repo_root(&cwd)?;
            let goal = std::fs::read_to_string(crate::goal::goal_path(&repo))
                .ok()
                .and_then(|md| crate::goal::parse_goal(&md));
            let changes = crate::delivery::changes(&repo);
            Some((repo, goal, changes))
        })
        .await
        .ok()
        .flatten();
        if let Some((repo, goal, changes)) = loaded {
            emit(Message::Loaded(project_id, repo, tab_id, goal, changes));
        }
    });
}
```

- [ ] **Step 4: 声明模块**

`crates/dozer-app/src/extensions.rs`,按字母序插入(在 `acceptance` 应排最前):

```rust
pub mod acceptance;
pub mod browser;
pub mod files;
pub mod git_log;
pub mod todo;
pub mod usage;
```

- [ ] **Step 5: 编译确认**

```bash
cargo build -p dozer-app
```

Expected: 编译通过。`update`/`spawn_open`/`WorkspaceState` 目前未被内核调用,
`dead_code` 警告可接受,不允许报错。`set_session_for_test` 标了 `#[cfg(test)]`,
Step 6 会用到。

- [ ] **Step 6: 新增单测**

在 `acceptance.rs` 文件末尾加 `#[cfg(test)] mod tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_session() -> AcceptanceSession {
        AcceptanceSession {
            repo: PathBuf::from("/tmp/repo"),
            source_tab_id: 0,
            goal: None,
            changes: vec![
                FileChange { path: "a.rs".into(), added: Some(1), removed: None },
                FileChange { path: "b.rs".into(), added: Some(2), removed: Some(1) },
            ],
            checked: vec![],
            expanded: HashSet::new(),
            diffs: HashMap::new(),
            comment: String::new(),
            comment_editing: false,
            error: None,
            accepted_version: None,
        }
    }

    fn ws_with_session() -> WorkspaceState {
        let mut ws_state = WorkspaceState::default();
        ws_state.set_session_for_test(sample_session());
        ws_state
    }

    #[tokio::test]
    async fn toggle_flips_checked() {
        let mut ws_state = WorkspaceState::default();
        ws_state.set_session_for_test(AcceptanceSession {
            checked: vec![false, false],
            ..sample_session()
        });
        let handle = tokio::runtime::Handle::current();
        update(&mut ws_state, Message::Toggle(1), 1, &test_client(), &handle, |_| {});
        assert_eq!(ws_state.session().unwrap().checked, vec![false, true]);
    }

    #[tokio::test]
    async fn toggle_diff_expands_then_collapses_without_reload() {
        let mut ws_state = ws_with_session();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            Message::ToggleDiff(0),
            1,
            &test_client(),
            &handle,
            |_| panic!("异步任务在测试里不需要真正跑,这里只断言同步状态"),
        );
        // 注意:上面这行会真的 spawn 一个读 /tmp/repo 的异步任务(大概率
        // 失败,emit 不会被调用,因为 /tmp/repo 不是真实仓库)——测试只关心
        // 同步部分:`expanded` 立刻加入 0。
        assert!(ws_state.session().unwrap().expanded_contains(0));
        update(&mut ws_state, Message::ToggleDiff(0), 1, &test_client(), &handle, |_| {});
        assert!(!ws_state.session().unwrap().expanded_contains(0), "第二次点收起");
    }

    #[tokio::test]
    async fn diff_loaded_writes_cache() {
        let mut ws_state = ws_with_session();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            Message::DiffLoaded(1, 0, Ok("+line".to_string())),
            1,
            &test_client(),
            &handle,
            |_| {},
        );
        assert_eq!(
            ws_state.session().unwrap().diff_for(0),
            Some(Ok("+line".to_string())).as_ref()
        );
    }

    #[tokio::test]
    async fn done_ok_sets_accepted_version_err_sets_error() {
        let mut ws_state = ws_with_session();
        let handle = tokio::runtime::Handle::current();
        update(&mut ws_state, Message::Done(1, Ok(3)), 1, &test_client(), &handle, |_| {});
        assert_eq!(ws_state.session().unwrap().accepted_version(), Some(3));
        let mut ws_state = ws_with_session();
        update(
            &mut ws_state,
            Message::Done(1, Err("boom".to_string())),
            1,
            &test_client(),
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.session().unwrap().error(), Some("boom"));
    }

    #[tokio::test]
    #[should_panic(expected = "由内核拦截处理")]
    async fn open_reaching_update_panics() {
        let mut ws_state = WorkspaceState::default();
        let handle = tokio::runtime::Handle::current();
        update(&mut ws_state, Message::Open(0), 1, &test_client(), &handle, |_| {});
    }

    #[tokio::test]
    #[should_panic(expected = "由内核拦截处理")]
    async fn reject_reaching_update_panics() {
        let mut ws_state = WorkspaceState::default();
        let handle = tokio::runtime::Handle::current();
        update(&mut ws_state, Message::Reject, 1, &test_client(), &handle, |_| {});
    }
}
```

上面测试用到了三个还没定义的 `AcceptanceSession` 只读访问器
(`expanded_contains`/`diff_for`/`accepted_version`/`error`)——回到 Step 1,给
`AcceptanceSession` 补一个 `impl` 块:

```rust
impl AcceptanceSession {
    pub fn expanded_contains(&self, i: usize) -> bool {
        self.expanded.contains(&i)
    }

    pub fn diff_for(&self, i: usize) -> Option<&Result<String, String>> {
        self.diffs.get(&i)
    }

    pub fn accepted_version(&self) -> Option<u32> {
        self.accepted_version
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// 供内核 `Reject` 拦截处理读取——要往哪个来源会话写打回意见。
    pub fn source_tab_id(&self) -> usize {
        self.source_tab_id
    }

    /// 供内核 `Reject` 拦截处理读取——打回意见文本。
    pub fn comment(&self) -> &str {
        &self.comment
    }
}
```

上面测试用的 `test_client()` 辅助——`extensions/browser.rs` 测试模块(254 行)已经有
一个同样用途的 `client_for_test()`,连到一个不存在的 unix socket 路径上(测试只需要
一个能编译通过的 `Client` 值,不需要真的连上 daemon,因为这些测试断言的都是同步状态
变化,不会真的等待网络调用完成)。在 `acceptance.rs` 的 `mod tests` 里加同款:

```rust
fn test_client() -> Client {
    Client::new(std::path::PathBuf::from(
        "/tmp/dozer-acceptance-test-nonexistent.sock",
    ))
}
```

- [ ] **Step 7: 跑测试**

```bash
cargo test -p dozer-app acceptance::
```

Expected: 全部新增测试 PASS(`toggle_diff_expands_then_collapses_without_reload`
可能因为真的 spawn 了一个失败的异步任务而在测试输出里看到一条 tracing 警告或者
`emit` 从未被调用,这是预期行为,不影响断言通过)。

- [ ] **Step 8: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/extensions/acceptance.rs crates/dozer-app/src/extensions.rs
git commit -m "feat(dozer-app): add acceptance Message/WorkspaceState/update/spawn_open"
```

---

### Task 4: `extensions::acceptance::view`

**Files:**
- Modify: `crates/dozer-app/src/extensions/acceptance.rs`

**Interfaces:**
- Consumes:Task 3 的 `WorkspaceState`/`AcceptanceSession`/`Message`。
- Produces:`pub fn view<'a>(ws_state: &'a WorkspaceState, width: Length, outer:
  Border) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>`。

- [ ] **Step 1: 文件顶部补齐 `view` 需要的 import**

```rust
use crate::theme;
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{button, column, container, row, text};
```

- [ ] **Step 2: `view` 主体**

```rust
/// 面板主入口。`session` 为 `None` 时是空态(还没有进行中的验收);有
/// `session` 且 `accepted_version.is_some()` 时只显示"已沉淀"提示(现有
/// `acceptance_content` 提前 return 那部分逻辑)。
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(session) = ws_state.session() else {
        return container(
            text("没有待验收的交付——完成一轮 agent 会话后,点这个图标就能看到")
                .size(theme::font::subtitle())
                .color(theme::color::DIM),
        )
        .width(width)
        .height(Length::Fill)
        .padding(20)
        .into();
    };

    let mut content = column![].spacing(12).padding(14);

    if let Some(n) = session.accepted_version {
        content = content.push(
            text(format!("✓ 已沉淀 v{n}"))
                .size(theme::font::title())
                .color(theme::color::GOLD),
        );
        return container(content)
            .width(width)
            .height(Length::Fill)
            .into();
    }

    match &session.goal {
        Some(g) => {
            content = content.push(
                text(g.title.clone())
                    .size(theme::font::title())
                    .color(theme::color::CREAM),
            );
            for (i, c) in g.criteria.iter().enumerate() {
                let checked = session.checked.get(i).copied().unwrap_or(false);
                content = content.push(
                    button(
                        text(format!("{} {c}", if checked { "✓" } else { "○" }))
                            .size(theme::font::body())
                            .color(if checked { theme::color::GOLD } else { theme::color::BODY }),
                    )
                    .on_press(Message::Toggle(i))
                    .style(|_t, _s| button::Style {
                        background: None,
                        text_color: theme::color::BODY,
                        ..button::Style::default()
                    }),
                );
            }
        }
        None => {
            content = content.push(
                text("未定标——先在仓库写 .dozer/goal.md（首行目标,\n- [ ] 列表为标准）")
                    .size(theme::font::body())
                    .color(theme::color::DIM),
            );
        }
    }

    content = content.push(
        text("变更文件")
            .size(theme::font::body())
            .color(theme::color::DIM),
    );
    for (i, fc) in session.changes.iter().enumerate() {
        let line = match (fc.added, fc.removed) {
            (Some(a), Some(r)) => format!("{}  +{a} −{r}", fc.path),
            _ => format!("{}  (新)", fc.path),
        };
        content = content.push(
            button(text(line).size(theme::font::body()).color(theme::color::CYAN))
                .on_press(Message::ToggleDiff(i))
                .width(Length::Fill)
                .style(|_t, _s| button::Style {
                    background: None,
                    text_color: theme::color::CYAN,
                    ..button::Style::default()
                }),
        );
        if session.expanded.contains(&i) {
            content = content.push(diff_view(session.diffs.get(&i)));
        }
    }

    let editing = session.comment_editing;
    let comment_text = if editing {
        format!("{}▏", session.comment)
    } else if session.comment.is_empty() {
        "验收意见…（打回时注回会话）".to_string()
    } else {
        session.comment.clone()
    };
    content = content.push(
        button(
            text(comment_text)
                .size(theme::font::body())
                .color(if editing { theme::color::CREAM } else { theme::color::DIM }),
        )
        .on_press(Message::CommentClick)
        .width(Length::Fill)
        .style(move |_t, _s| button::Style {
            background: Some(theme::color::TERM_BG.into()),
            text_color: theme::color::CREAM,
            border: Border {
                color: if editing { theme::color::GOLD } else { theme::color::BORDER },
                width: 1.0,
                radius: 2.0.into(),
            },
            ..button::Style::default()
        }),
    );

    content = content.push(
        row![
            button(text("通过·沉淀").size(theme::font::body()).color(theme::color::BG))
                .on_press(Message::Accept)
                .style(|_t, _s| button::Style {
                    background: Some(theme::color::GOLD.into()),
                    text_color: theme::color::BG,
                    border: Border { color: theme::color::GOLD, width: 1.0, radius: 2.0.into() },
                    ..button::Style::default()
                }),
            button(text("打回并注回").size(theme::font::body()).color(theme::color::RED))
                .on_press(Message::Reject)
                .style(|_t, _s| button::Style {
                    background: None,
                    text_color: theme::color::RED,
                    border: Border { color: theme::color::RED, width: 1.0, radius: 2.0.into() },
                    ..button::Style::default()
                }),
        ]
        .spacing(8),
    );

    if let Some(err) = &session.error {
        content = content.push(
            text(format!("⚠ {err}"))
                .size(theme::font::body())
                .color(theme::color::RED),
        );
    }

    container(content)
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| iced_widget::container::Style {
            background: None,
            border: outer,
            ..iced_widget::container::Style::default()
        })
        .into()
}

/// 手风琴展开的 diff 内容:`Ok(patch)` 按行首字符 `+`/`-`/` ` 分三色渲染,
/// `Err(e)` 显示红字,`None`(还没加载完)显示"加载中…"。
fn diff_view<'a>(
    diff: Option<&'a Result<String, String>>,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    match diff {
        None => text("加载中…").size(theme::font::caption()).color(theme::color::DIM).into(),
        Some(Err(e)) => text(format!("⚠ {e}"))
            .size(theme::font::caption())
            .color(theme::color::RED)
            .into(),
        Some(Ok(patch)) => {
            let mut col = column![].spacing(0).padding([4, 12]);
            for line in patch.lines() {
                let color = if line.starts_with('+') {
                    theme::color::GREEN
                } else if line.starts_with('-') {
                    theme::color::RED
                } else {
                    theme::color::DIM
                };
                col = col.push(
                    text(line.to_string())
                        .size(theme::font::caption_sm())
                        .color(color)
                        .font(iced_widget::core::Font::MONOSPACE)
                        .line_height(LineHeight::Relative(1.3)),
                );
            }
            container(col)
                .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: Some(theme::color::TERM_BG.into()),
                    ..iced_widget::container::Style::default()
                })
                .into()
        }
    }
}
```

- [ ] **Step 3: 编译,逐条修正**

```bash
cargo build -p dozer-app
```

Expected: 逐条修正类型/字段名不一致的地方(比如 `theme::color`/`theme::font` 的
具体常量名要跟 `crates/dozer-app/src/theme` 实际导出的对上——写代码时如果某个颜色/
字号 token 不存在,查 `theme::color`/`theme::font` 模块现有的常量列表换成实际存在
的那个,不要臆造新 token)。

- [ ] **Step 4: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/acceptance.rs
git commit -m "feat(dozer-app): add acceptance::view with self-contained diff accordion"
```

---

### Task 5: 内核接线 A——`RightView`/`RailButton`/图标 rail/字段合并

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes:Task 2 的 `icons::IconKind::BadgeCheck`,Task 3 的
  `acceptance::WorkspaceState`。

- [ ] **Step 1: `RightView`/`RailButton` 新增变体**

`RightView` 枚举(现约 92 行)加 `Acceptance`:

```rust
pub enum RightView {
    Agent,
    Conversations,
    Usage,
    Acceptance,
}
```

`RailButton` 枚举(现约 102 行)加 `RightAcceptance`:

```rust
pub enum RailButton {
    LeftFiles,
    LeftWeb,
    LeftGit,
    LeftTodo,
    RightAgent,
    RightConversations,
    RightUsage,
    RightAcceptance,
}
```

- [ ] **Step 2: `Workspace` 字段合并**

`pub struct Workspace { .. }` 里删除:

```rust
acceptance: Option<AcceptanceView>,
```

加:

```rust
/// 验收面板 per-project 状态——见 `extensions::acceptance::WorkspaceState`。
acceptance: acceptance::WorkspaceState,
```

`Workspace::empty_for_project_placeholder()` 里 `acceptance: None,` 改成
`acceptance: acceptance::WorkspaceState::default(),`。

- [ ] **Step 3: `use` 引入**

`workspace.rs` 顶部按字母序加:

```rust
use crate::extensions::acceptance;
```

- [ ] **Step 4: `right_icon_rail` 新增第 4 个按钮 + 徽标**

```rust
MouseArea::new(rail_icon_button(
    icons::IconKind::BarChart3,
    app.right_view == RightView::Usage && right_open,
    app.hover_progress(HoverId::Rail(RailButton::RightUsage)),
    Message::RightIconSelect(RightView::Usage),
))
.on_enter(Message::Hover(HoverId::Rail(RailButton::RightUsage), true))
.on_exit(Message::Hover(HoverId::Rail(RailButton::RightUsage), false)),
```

这四行之后加第 4 个按钮(需要在图标右上角叠一个金点徽标,用 `iced_widget::Stack`
包一层——`delivery_pending` 读当前项目、当前激活 tab):

```rust
{
    let pending = app
        .active_workspace()
        .and_then(|ws| ws.tabs.get(ws.active))
        .map(|t| t.delivery_pending)
        .unwrap_or(false);
    let base = MouseArea::new(rail_icon_button(
        icons::IconKind::BadgeCheck,
        app.right_view == RightView::Acceptance && right_open,
        app.hover_progress(HoverId::Rail(RailButton::RightAcceptance)),
        Message::RightIconSelect(RightView::Acceptance),
    ))
    .on_enter(Message::Hover(HoverId::Rail(RailButton::RightAcceptance), true))
    .on_exit(Message::Hover(HoverId::Rail(RailButton::RightAcceptance), false));
    if pending {
        iced_widget::stack![
            base,
            container(iced_widget::Space::new())
                .width(Length::Fixed(8.0))
                .height(Length::Fixed(8.0))
                .style(|_t: &iced_widget::Theme| container::Style {
                    background: Some(theme::color::GOLD.into()),
                    border: Border { radius: 4.0.into(), ..Border::default() },
                    ..container::Style::default()
                })
        ]
        .into()
    } else {
        base.into()
    }
},
```

（`ws.tabs`/`SessionTab.delivery_pending` 是私有字段,`app.active_workspace()`
返回 `Option<&Workspace>`——确认这条链路在 `right_icon_rail(app: &App)` 的作用域内
可以直接这样访问；若 `tabs`/`active`/`delivery_pending` 字段可见性不允许从这个
自由函数访问,说明它们本来就是同文件私有字段,`right_icon_rail` 本来就在
`workspace.rs` 同一个文件里,能访问。写代码时若编译报可见性错误,再排查具体是哪个
字段。）

`Stack` 需要的 `iced_widget::stack!` 宏——确认文件顶部 `use iced_widget::{..}` 里
已经有 `stack`(前面浮层判断链已经用过 `stack![base, dismiss, ..]`,同一个宏,不用
新增 import)。

- [ ] **Step 5: `RightIconSelect` 切到验收视图时触发 `Open`**

设计文档目标 #7 要求"点图标随时能看,不需要先看到横幅",这一步是真正让点击触发加载
的地方——现有 `Message::RightIconSelect(v)` 处理器(现约 3575-3590 行)里,切到
`RightView::Usage` 时会顺带触发一次刷新;`RightView::Acceptance` 要同样处理,但
触发的是 `acceptance::Message::Open(tab_id)`,`tab_id` 取当前激活 tab:

```rust
Message::RightIconSelect(v) => {
    if self.right_view == v {
        if !self.left_collapsed {
            self.right_collapsed = !self.right_collapsed;
        }
    } else {
        self.right_view = v;
        self.right_collapsed = false;
        if v == RightView::Usage {
            self.with_focused_project(|ws, io| {
                ws.usage.set_loading(true);
                ws.spawn_usage_refresh(io);
            });
        } else if v == RightView::Acceptance {
            let tab_id = self
                .active_workspace()
                .and_then(|ws| ws.tabs.get(ws.active))
                .map(|t| t.tab_id);
            if let Some(tab_id) = tab_id {
                self.update(Message::Acceptance(acceptance::Message::Open(tab_id)));
            }
        }
    }
    self.maximized = None;
    self.on_shell_layout_changed();
}
```

（`self.active_workspace()` 先取一份不可变借用拿到 `tab_id` 就结束,借用已经释放,
之后才调用 `self.update(..)`(可变借用)——不能在同一个 `with_focused_project`
闭包里既借 `ws` 又调用 `self.update`,那是两层可变借用冲突。`ws.tabs`/
`SessionTab.tab_id` 是同文件私有字段,`active_workspace()` 就定义在
`workspace.rs` 里,直接访问没有可见性问题。`self.update(Message::Acceptance(..))`
这种"处理一条消息时递归调用 `self.update` 处理另一条"的写法,`Message::GitLog`
分支处理 `SnapshotLoaded` 落地后恢复选中提交那里已经用过同一手法,不是新模式。）

- [ ] **Step 6: 编译确认**

```bash
cargo build -p dozer-app
```

Expected: 报一堆"缺 match 分支"/"字段类型不对"的错误(`RightView::Acceptance` 加进
穷举枚举后,所有 `match self.right_view` 的地方要补分支;`ws.acceptance` 从
`Option<AcceptanceView>` 变成 `acceptance::WorkspaceState` 后,所有直接访问
`ws.acceptance` 字段的地方要改)——这些留到 Task 6 逐条修正,本任务先确认到这一步
即可,不需要现在就修完(Task 6 会接着改 `update()`/`view()` 剩余部分)。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): add RightView::Acceptance rail icon with delivery badge"
```

（这一步允许 `cargo build` 仍然报错——中间状态提交,Task 6 会继续改到编译通过。如果
你的工作流不允许中间状态提交,合并 Task 5/6 一次性做完再提交,不要因为这条约束卡在
这里。）

---

### Task 6: 内核接线 B——`update()` 路由 + 删横幅 + 删旧函数

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes:Task 3 的 `acceptance::Message`/`acceptance::update`/
  `acceptance::spawn_open`。

- [ ] **Step 1: 顶层 `Message` 枚举**

删除 8 个变体:`AcceptanceOpen`/`AcceptanceLoaded`/`AcceptanceToggle`/
`AcceptanceCommentClick`/`AcceptanceCommentEvent`/`AcceptanceAccept`/
`AcceptanceReject`/`AcceptanceDone`,加:

```rust
/// 验收面板的全部消息,内核只转发不解读——见 `extensions::acceptance::Message`。
Acceptance(acceptance::Message),
```

- [ ] **Step 2: `update()` 里 Acceptance 相关分支**

删除现有 `Message::AcceptanceOpen`/`AcceptanceLoaded`/`AcceptanceToggle`/
`AcceptanceCommentClick`/`AcceptanceCommentEvent`/`AcceptanceAccept`/
`AcceptanceReject`/`AcceptanceDone` 8 支(现约 3196-3309 行区间),加 4 支:

```rust
Message::Acceptance(acceptance::Message::Open(tab_id)) => {
    self.with_focused_project(|ws, io| {
        let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
        let Some(project_id) = ws.project_id() else { return };
        let Some(tab) = ws.tab_by_id_mut(tab_id) else { return };
        tab.delivery_pending = false;
        let cwd = effective_project_repo(active_repo.as_deref(), &tab.effective_cwd());
        let handle = io.handle.clone();
        let proxy = io.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Acceptance(m));
        };
        acceptance::spawn_open(project_id, tab_id, cwd, &handle, emit);
    });
}
Message::Acceptance(acceptance::Message::Reject) => {
    self.with_focused_project(|ws, io| {
        let Some(session) = ws.acceptance.session() else { return };
        let comment = session.comment().trim().to_string();
        let source = session.source_tab_id();
        let target = ws.tabs.iter().find(|t| t.tab_id == source);
        let Some(tab) = target.filter(|t| t.alive) else {
            // 会话已结束,意见无处可注——留住当前 session,不清空,让用户
            // 看到错误(现有 `acceptance_reject` 的降级路径)。
            ws.acceptance.set_error("会话已结束,意见无处可注".to_string());
            return;
        };
        let id = tab.info.id.clone();
        let client = io.client.clone();
        let text_out = format!("[Dozer 验收打回] {comment}\n");
        io.handle.spawn(async move {
            if let Err(e) = client.write(&id, text_out.as_bytes()).await {
                tracing::warn!("打回注回失败: {e}");
            }
        });
        ws.acceptance.clear_session();
    });
}
Message::Acceptance(
    msg @ (acceptance::Message::Loaded(project_id, ..)
    | acceptance::Message::DiffLoaded(project_id, ..)
    | acceptance::Message::Done(project_id, ..)),
) => {
    // 判断"这次是不是通过成功"要在 `msg` 被 `move` 进闭包之前算好
    // (用 `&msg` 引用匹配,不消耗它;闭包里 `acceptance::update` 会真正
    // 拿走 `msg` 的所有权),否则会撞上"用后借用"的编译错误。
    let is_accept_ok = matches!(&msg, acceptance::Message::Done(_, Ok(_)));
    self.with_project(project_id, move |ws, io| {
        let client = io.client.clone();
        let handle = io.handle.clone();
        let proxy = io.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Acceptance(m));
        };
        acceptance::update(&mut ws.acceptance, msg, project_id, &client, &handle, emit);
        if is_accept_ok {
            ws.spawn_acceptance_count_refresh(io);
        }
    });
}
Message::Acceptance(msg) => {
    let Some(project_id) = self.active_project_id else { return };
    self.with_focused_project(|ws, io| {
        let client = io.client.clone();
        let handle = io.handle.clone();
        let proxy = io.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Acceptance(m));
        };
        acceptance::update(&mut ws.acceptance, msg, project_id, &client, &handle, emit);
    });
}
```

`ws.acceptance.session()` 返回 `Option<&AcceptanceSession>`,`session.comment()`/
`session.source_tab_id()`/`ws.acceptance.set_error(..)` 都是 Task 3 已经加好的
访问器,直接用。

- [ ] **Step 3: `terminal_pane` 删除交付横幅**

删除现有"交付横幅（spec P1f D3）"那一段(现约 6819-6850 行区间,含
`Message::AcceptanceOpen(tab.tab_id)` 那个按钮)。

- [ ] **Step 4: `right_panel_area` 加 `RightView::Acceptance` 分支**

在 `RightView::Usage => usage::view(..)` 分支旁边加:

```rust
RightView::Acceptance => acceptance::view(&ws.acceptance, Length::Fill, zone_pane_border(zone, ac))
    .map(Message::Acceptance),
```

- [ ] **Step 5: 删除旧函数/类型**

删除:`AcceptanceView` 结构体定义、`Workspace::acceptance_accept`、
`Workspace::acceptance_reject`、`acceptance_content` 函数、`banner_text` 函数、
`criteria_line` 函数、`file_change_line` 函数(连同它们对应的现有测试
`banner_text_for_pending`/`criteria_check_line_renders_gold_check`/
`file_change_line_formats_counts`——这三个测试的断言逻辑已经在 Task 3 的
`extensions::acceptance` 测试里用不同形式覆盖,若发现某个具体断言没被覆盖,照抄
过去,不要丢测试覆盖)。

- [ ] **Step 6: 编译,逐条修正**

```bash
cargo build -p dozer-app
```

Expected: 先会有一长串报错。逐条处理:
- `match self.right_view { .. }` 缺 `RightView::Acceptance` 分支的地方补上
  (参考现有 `RightView::Usage` 分支怎么处理,同款处理,比如
  `right_view_pairs_with_agent_terminal` 之类的辅助函数如果有穷举 `match`)。
- 直接访问 `ws.acceptance`(当作 `Option<AcceptanceView>`)字段/方法的地方,改成
  `ws.acceptance.session()`/新增的访问器。
- `Message::AcceptanceOpen(tab.tab_id)` 之类残留的旧消息构造点,改成
  `Message::Acceptance(acceptance::Message::Open(tab.tab_id))`(若还有,比如
  某处快捷键/测试 fixture)。

- [ ] **Step 7: 全量测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt
```

Expected: 全绿。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(dozer-app): route Acceptance messages through extensions::acceptance"
```

---

### Task 7: `preview.rs` 精简

**Files:**
- Modify: `crates/dozer-app/src/preview.rs`

- [ ] **Step 1: 删除 `TabKind::Acceptance` 及相关方法**

`TabKind` 枚举删除 `Acceptance` 变体,只留 `File(PathBuf)`。删除
`PreviewPane::open_acceptance`、`PreviewPane::acceptance_active`。

`active_webview_id` 方法:

```rust
pub fn active_webview_id(&self) -> Option<usize> {
    self.tabs.get(self.active).and_then(|t| match t.kind {
        TabKind::File(_) => Some(t.id),
    })
}
```

（`match` 现在只有一个分支,`TabKind::Acceptance => None` 那行删掉;如果 Rust 提示
单分支 `match` 可以简化成别的写法,按 clippy 建议改,不强制保留 `match` 语法。）

`desired_webviews` 方法删除 `overlay_active` 相关逻辑:

```rust
pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
    self.tabs
        .iter()
        .enumerate()
        .map(|(idx, tab)| {
            let TabKind::File(path) = &tab.kind;
            let mut u = flyfish_url(path);
            if tab.reload_nonce > 0 {
                u.push_str(&format!("&_r={}", tab.reload_nonce));
            }
            WebviewSpec {
                id: tab.id,
                url: u,
                visible: idx == self.active,
            }
        })
        .collect()
}
```

（原来是 `filter_map` 因为 `TabKind::Acceptance` 分支要 `return None`;现在
`TabKind` 只有一种,改成 `map` 更直接。`let TabKind::File(path) = &tab.kind;` 这种
"只有一个变体,直接解构不用 match"的写法如果 Rust 版本/clippy 不认,改回
`match &tab.kind { TabKind::File(path) => { .. } }` 单分支写法,行为一致。）

- [ ] **Step 2: 删除相关测试**

删除 `acceptance_tab_produces_no_webview_and_hides_others` 测试。

- [ ] **Step 3: 编译,逐条修正**

```bash
cargo build -p dozer-app
```

Expected: 若 `workspace.rs` 还有地方引用 `crate::preview::TabKind::Acceptance`
(Task 6 应该已经清完,这里是保险检查),报错位置回 Task 6 补删。

- [ ] **Step 4: 全量测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/preview.rs
git commit -m "refactor(dozer-app): drop TabKind::Acceptance now that it has its own RightView"
```

---

### Task 8: 全量校验与人工验收

**Files:** 无代码改动。

- [ ] **Step 1: 全 workspace 构建 + 测试 + clippy + fmt**

```bash
cargo build
cargo test -p dozer-app
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全绿。

- [ ] **Step 2: 人工验收(`cargo run -p dozer-app`)**

对照设计文档逐项走一遍:
- 完成一轮 agent 会话产生交付后,右侧新图标(BadgeCheck)出现金点徽标,终端面板里
  **不再**出现横幅。
- 点新图标随时能进入验收(不需要先触发交付/不需要先看到任何提示)。
- 有 `.dozer/goal.md` 时显示目标标题 + 标准复选框,点击能勾选/取消。
- 没有 `.dozer/goal.md` 时显示"未定标"提示,不阻塞后续操作。
- 变更文件列表:点文件名展开 diff(新增文件全绿、删除的行标红、修改的文件红绿混合),
  再点收起;左侧文件预览**不受影响**,不会因为点验收面板的变更文件而联动跳转。
- 验收意见框:点击进入编辑,输入文字,提交/取消。
- 点"通过·沉淀":显示"✓ 已沉淀 v<n>",项目卡"N 次验收"副行数字增加。
- 点"打回并注回":对应来源终端会话收到 `[Dozer 验收打回] ...` 文本,验收面板回到
  空态,图标徽标消失(因为 `delivery_pending` 已经在 `Open` 阶段清过)。
- 来源会话已结束时点"打回并注回":面板显示"会话已结束,意见无处可注"错误,不清空
  当前会话内容。
- 切换项目页签:验收状态/图标徽标各自独立,不串项目。
- 普通文件预览(不涉及验收)功能不受影响——`preview.rs` 精简后打开文件、编辑保存、
  webview 显隐都正常。

- [ ] **Step 3: 确认没有遗留未提交的改动**

```bash
git status
```

Expected: 干净,无未跟踪/未暂存改动(`.dozer/` 目录等项目运行时产物除外)。
