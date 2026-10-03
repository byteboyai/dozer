# bytegit P4:变更监听迁移 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 `bytegit` 增加 `watch` feature(默认关,引入 `notify`),提供 `bytegit::watch(root, opts, on_change)`,并让 dozer 的 `git_watch.rs` 整个删除、改用它:路径分类、`.git` 引用文件识别、debounce 都进 `bytegit`,「哪些目录不关心」由调用方传入(dozer 传 `project::HIDDEN`),行为保持等价。

**Architecture:** 前半在 `bytegit` 仓库加 `watch.rs`:一个纯函数 `classify`(路径是否相关、属于哪一类)、一个把 notify 事件折成批的 `batch_of`、一个不依赖 tokio 的 `debounce_loop`(标准线程 + `mpsc` 的安静期 debounce)。这三块都不碰文件系统(除 `batch_of` 的路径规范化),所以能用确定性测试覆盖;真实文件系统只留四条"事件到了才断言"的端到端测试。后半在 dozer 的独立 worktree 里:**先在旧的 `git_watch.rs` 上补刻画测试并确认通过**(旧分类函数是纯函数,可以直接对着旧实现断言),再删掉 `git_watch.rs`,把 `Message::ProjectFsChanged`、`Workspace.git_watch`、`project_fs_changed` 切到 `bytegit::GitChange`/`WatchHandle`。分类断言(含新增的 6 条)在 bytegit 里原样保留,同一批断言在旧、新实现上都通过。

**Tech Stack:** Rust edition 2024、`notify 8`(仅 bytegit 的 `watch` feature)、标准库线程与 `mpsc`;dozer 侧 `bytegit` 以 git tag 依赖并开启 `watch` feature。

**Spec:** `docs/superpowers/specs/2026-10-02-bytegit-design.md`(§4.5、§6 的 P4、§8 的 O3)。前序计划:P0–P2(已完成、已合并 `main`,bytegit `v0.3.0`);P3 `2026-10-03-bytegit-p3-stats-churn.md`(与本计划互不依赖,见"前置条件")。

**本计划中的代码已在草稿里完整跑过:** bytegit 默认 84 个测试、`--all-features` 107 个测试通过(新增 23 个,其中 19 个不依赖文件系统事件的投递),`cargo clippy --all-targets`(默认与 `--all-features`)与 fmt 干净;依赖事件投递的两条端到端测试(debounce、引用变化)在本机**确实收到了 FSEvents 事件并通过**(不是靠"收不到就跳过")。dozer 侧在一份克隆里实际编译:旧 `git_watch.rs` 的 13 个测试(原有 7 个 + 新增 6 个刻画测试)在**旧实现上**全部通过;切换后 `cargo test -p dozer-app` 通过数比旧实现少 13 个(正好是迁到 bytegit 的那 13 个),唯一失败仍是"已知基线"里的 `delete_confirm_spec_reflects_pending_target`,`cargo clippy -p dozer-app --all-targets` 的诊断与旧实现逐条相同。bytegit 的 7 个变异检验都会让对应测试失败(见 Task 1)。执行者照抄即可;结果与此不符先停下排查,不要改测试迎合实现。

## 前置条件

- bytegit 仓库 `main` 当前是 `v0.3.0`(P2 已发布)。P4 与 P3 **互不依赖**(P3 动 `stats.rs`/`history.rs`/`testutil.rs`,P4 动 `watch.rs`/`error.rs`/`Cargo.toml`;只有 `lib.rs` 与 `Cargo.toml` 的版本号两个文件会碰到同一处)。
- **版本号:** 本计划按"P3 尚未发布"写,发 `v0.4.0`。P3 的计划同样写的是 `v0.4.0`;**两者谁后执行,谁把自己计划里的 `0.4.0`/`v0.4.0` 全部顺延为 `0.5.0`/`v0.5.0`**(`grep -n "0\.4\.0" <计划文件>` 能找全),dozer 侧的 `tag` 同理。若 P3 先发布了,本计划里 bytegit 的测试数基线也会变(P3 新增 20 个),以"相对增量 +23"为准。
- dozer 侧 P1、P2 已合并 `main`,P4 分支从 `main` 开。

## Global Constraints

- bytegit:Rust edition `2024`;公开 API 不得出现 `git2::*`、`notify::*`;`watch` 只用标准线程,**不依赖 tokio/iced**;`watch` feature 默认关,`cargo build`(不开 feature)的依赖树里不得出现 `notify`。
- dozer 侧**所有命令都在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytegit-p4/...`);主 checkout 常有并发会话与未提交改动,不得在其上改代码或 `git add -A`。提交前 `git status` 与 `git diff --cached --stat` 看全貌,只 `git add` 指定路径。
- dozer 的 `bytegit` 依赖一律用 tag:`bytegit = { git = "https://github.com/byteboyai/bytegit", tag = "v0.4.0", features = ["watch"] }`;本地联调的 `[patch]` 只放 `.cargo/config.toml`,**不提交**。带着本地 `[patch]` 构建会让 `Cargo.lock` 里 `bytegit`/`byteui` 的 `source = "git+..."` 行消失;提交 `Cargo.lock` 前必须把 `.cargo/config.toml` 里 `bytegit` 的 `[patch]` 段暂时注释掉再 `cargo update -p bytegit`(见 Task 4 Step 1),确认 diff 里 `source` 行都在、只有 bytegit 的 tag 与 dozer-app 的依赖列表(少了 `notify`)变了。
- 迁移阶段**不得改变用户可见行为**。本计划有意的差异只有"待决事项"里的 D8、D9(都只让结果更准确)。
- 新增的 git 相关读取一律调 `bytegit`(见 `CLAUDE.md`)。`dozer-app` 不再直接依赖 `notify`。
- dozer 门禁:触碰的文件不产生新诊断、基线之外无新失败(见"已知基线")。`cargo fmt -p dozer-app` 后用 `git status --short` 确认**只有本任务触碰的文件**变了;fmt 动了别的文件就 `git checkout` 回去。
- 提交信息用 `feat:`/`fix:`/`refactor:`/`test:`/`docs:` 前缀;提交末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知基线

沿用 P2/P3 计划的"已知基线":`extensions::files::tests::delete_confirm_spec_reflects_pending_target` 在 `main` 上就失败;`cargo build -p dozer-app` 有两条与本计划无关的 dead_code 警告(`usage/aggregate.rs::agent_token_share`、`git_accounts.rs::GitProvider::ALL`);`cargo clippy -p dozer-app --all-targets` 在 `workspace/state.rs` 里有两条既有诊断(迁移前后逐条相同,只是行号偏移)。**本计划触碰的文件不得新增诊断。**

## Review Focus

规格对下列输入没有逐条说明,但真实使用者会遇到;每条都由指明的任务里的测试固定下来(行为均在草稿里实测确认):

1. **忽略规则必须对任一层级生效、由调用方给,并且 `.git` 的特殊处理不依赖调用方。** monorepo 里 `packages/foo/node_modules`、每个子包各自的 `target/` 写入不能触发刷新;子模块/内嵌仓库(嵌套层的 `.git`)里的 `HEAD` 变化**不是**本仓库的引用变化;调用方传空规则时只有 `.git` 的内置处理仍然生效。bytegit Task 1 的 `nested_hidden_dirs_are_not_relevant`、`ignore_rules_are_defined_by_the_caller`,以及 dozer Task 3 在旧实现上固定的同一批断言。
2. **`.git` 下引用文件的识别边界。** `HEAD`、`index`、`packed-refs`、`refs/*` 算引用类;`index.lock`、`HEAD.lock`、`refsx` 不算;`refs/` 之下的 `.lock` 临时文件**算**引用类(迁移前就是这样,会多触发几次刷新,保持);仓库根的 `.git` 是文件(linked worktree/子模块的指针)时这个文件本身不上报。bytegit Task 1 的 `git_lock_files_and_lookalikes_are_not_refs_except_under_refs`、`a_dot_git_file_at_the_root_is_not_relevant`;dozer Task 3 对应的旧实现测试。
3. **debounce 是"安静期"语义,不是固定窗口。** 连续事件(间隔小于 debounce)不断延长这一批,最后一个事件之后静默满 debounce 才回调一次;一批里只要出现过引用变化,`refs_changed` 就为真;批与批之间隔了足够久就分别回调;路径去重。bytegit Task 1 的 `batches_arriving_within_the_window_merge_into_one_change`、`a_refs_change_anywhere_in_the_window_sets_refs_changed`、`a_quiet_period_splits_changes_into_separate_callbacks`(这三条不碰文件系统,不受平台事件投递的影响)。
4. **生命周期。** `WatchHandle` Drop 之后不能再有回调(项目页签切换时 dozer 会丢掉旧监听器,旧回调不能带着旧 project id 再发一次刷新);监听器启动失败(路径不存在、文件描述符耗尽)返回 `GitErrorKind::Io`,dozer 记一条 warn 并降级为"只在开项目/回合结束时刷新"。bytegit Task 1 的 `nothing_is_delivered_once_the_handle_is_stopped`、`no_callbacks_after_the_handle_is_dropped`、`watching_a_missing_path_is_an_io_error`。
5. **macOS 的路径规范化。** `/var/...` 是 `/private/var/...` 的符号链接,FSEvents 可能用任一形式回报;根目录与每个事件路径都要先规范化再比较(规范化失败——比如文件刚被删——就用原路径),否则整批事件被静默当成无关。bytegit Task 1 的 `event_paths_are_canonicalized_through_symlinks_before_classifying`(用真实符号链接,不依赖 FSEvents)。

## 待决事项

- **D8(知情):`GitChange.paths` 现在包含这一批里**所有**相关路径——工作区路径与 `.git` 引用文件的路径——去重并按字典序排序。** 迁移前 `FsChanges.paths` 是无序的,并且批里一旦遇到引用类路径就 `break`,同一事件里排在它后面的工作区路径会被丢掉。新行为是旧行为的超集;唯一的消费者 `reload_webviews_for(&changes.paths)` 只拿它去匹配已打开的预览页签,多出来的路径无害。
- **D9(知情):`WatchHandle` Drop 之后不再回调。** 迁移前 Drop 时若恰好有一批在途,debounce 任务还会再回调一次(`Message::ProjectFsChanged` 带着可能已经过期的 project id);新实现丢弃这一批。这是对 `workspace/state.rs` 里"旧 watcher 带着旧 project_id 继续在后台跑"那段注释所担心的情形的收紧,只会减少多余刷新。
- **D10(需要你定是否排期):`.git` 是文件的仓库(linked worktree、子模块)仍然收不到引用类变化。** 这是规格 O3 的已知缺口,**P4 不实现**(规格明确"v0.1 不强求"),只是把它写进 `watch` 的模块文档并用测试固定"那个 `.git` 文件本身不上报"。实际后果:项目目录是 linked worktree 时,在该 worktree 里切分支/提交不会触发 Git Log 重建,只有工作区文件变化会触发刷新——和迁移前完全一致。v2 引入 worktree 后这会变成真实问题(要同时监听 worktree 自己的 gitdir 与公共目录里的 `refs/`、`packed-refs`)。API 不需要改(`watch(root, ...)` 内部解析 gitdir 即可),所以可以以后单独补;请告知要不要排进后续阶段。

规格补充(Task 5 回写):`GitChange` 的语义(两个布尔 + 路径列表)、`IgnoreRules` 对 `.git` 的内置处理、`WatchOptions::new(debounce).ignore(rules)` 的构造方式、回调运行在 bytegit 自己的线程上、Drop 语义、`watch` feature 依赖 `notify 8`。另记一条可移植性提醒 **O15**:`watch` 与迁移前一样**不过滤 notify 的事件种类**——macOS 的 FSEvents 只报修改类事件,但 Linux 的 inotify 后端会报"文件被打开/读取"这类 Access 事件,届时 dozer 自己读文件(如 `git status`)可能反过来触发刷新。非 macOS 平台从未实际编译过(见 `CLAUDE.md`),等真要发布时再处理。

---

## File Structure

**bytegit 仓库**(`~/Projects/CoralProjects/byteboy/bytegit/`):

| 文件 | 变更 |
|------|------|
| `Cargo.toml` | 加 feature `watch` 与可选依赖 `notify = "8"`;版本 `0.3.0` → `0.4.0` |
| `src/watch.rs` | 新增(feature `watch`):`IgnoreRules`、`WatchOptions`、`GitChange`、`WatchHandle`、`watch()`,内部 `classify`/`batch_of`/`debounce_loop` |
| `src/error.rs` | 新增(feature `watch`):`From<notify::Error> for GitError`(归为 `Io`) |
| `src/lib.rs` | 声明 `mod watch;` 并再导出 |

**dozer 仓库**(专用 worktree `~/Projects/CoralProjects/byteboy/dozer-bytegit-p4/`,分支 `bytegit-p4`):

| 文件 | 变更 |
|------|------|
| `crates/dozer-app/src/git_watch.rs` | **删除**(Task 3 先在其上补刻画测试) |
| `crates/dozer-app/src/main.rs` | 去掉 `mod git_watch;` |
| `crates/dozer-app/src/app/message.rs` | `ProjectFsChanged(ProjectId, bytegit::GitChange)` |
| `crates/dozer-app/src/app/update.rs` | `project_fs_changed` 改读 `workdir_changed`/`refs_changed` |
| `crates/dozer-app/src/workspace/state.rs` | `git_watch: Option<bytegit::WatchHandle>`;`start_git_watch` 调 `bytegit::watch` |
| `crates/dozer-app/Cargo.toml`、`Cargo.lock` | `bytegit` 升到 `v0.4.0` 并开 `watch`;去掉直接的 `notify` 依赖 |
| 规格、`CLAUDE.md`、要求文档、盘点文档 | Task 5 同步 |

---

### Task 1: bytegit — `watch` feature

**Files:**
- Create: `src/watch.rs`
- Modify: `Cargo.toml`、`src/error.rs`、`src/lib.rs`

**Interfaces:**
- Consumes(已有):`GitError`/`GitErrorKind`
- Produces(Task 2、4 依赖,均在 feature `watch` 下):
  - `IgnoreRules::new<I, S>(names: I) -> IgnoreRules`(`I: IntoIterator<Item = S>, S: Into<String>`;`Default` 为空规则)
  - `WatchOptions::new(debounce: Duration) -> WatchOptions`、`WatchOptions::ignore(self, rules: IgnoreRules) -> WatchOptions`(`#[non_exhaustive]`,公开字段 `debounce`、`ignore`)
  - `struct GitChange { pub refs_changed: bool, pub workdir_changed: bool, pub paths: Vec<PathBuf> }`(`Debug + Clone + Default + PartialEq + Eq`)
  - `struct WatchHandle`(Drop 即停止)
  - `fn watch(root: &Path, opts: WatchOptions, on_change: impl FnMut(GitChange) + Send + 'static) -> Result<WatchHandle, GitError>`
  - `impl From<notify::Error> for GitError`(`Io`)

- [ ] **Step 1: `Cargo.toml` 加 feature 与依赖**

在 `[features]` 里 `testutil = ["dep:tempfile"]` 之后加:

```toml
# 文件系统变更监听:`watch()`。引入 `notify`;只用标准线程,不依赖 tokio。
watch = ["dep:notify"]
```

在 `[dependencies]` 里 `tempfile = { version = "3", optional = true }` 之后加:

```toml
notify = { version = "8", optional = true }
```

- [ ] **Step 2: `src/error.rs` 加 `From<notify::Error>`**

在 `impl From<std::io::Error> for GitError {` 之前加:

```rust
#[cfg(feature = "watch")]
impl From<notify::Error> for GitError {
    fn from(e: notify::Error) -> Self {
        Self::new(GitErrorKind::Io, e.to_string())
    }
}
```

- [ ] **Step 3: 写 `src/watch.rs`(实现 + 测试)**

```rust
//! 变更监听(feature `watch`):debounce 后上报"工作区文件变了"/"`.git` 引用类文件变了"。
//!
//! 只依赖 `notify` 与标准线程,不依赖 tokio。事件的发布(送进应用的消息循环/事件总线)
//! 不在这里:调用方在 `on_change` 里自己转发。
//!
//! **已知限制(规格 O3):** 只监听 `root` 目录树。`root/.git` 是目录时,其中的
//! `HEAD`/`index`/`packed-refs`/`refs/*` 会被识别;`.git` 是文件(linked worktree、子模块,
//! 指向别处的 gitdir)时,真实 gitdir 在 `root` 之外,**不会被监听**,只有工作区文件的变化会上报。
//! 以后补上不需要改这里的 API(监听目标改为解析后的 gitdir 即可)。

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use notify::Watcher;

use crate::{GitError, GitErrorKind};

/// 调用方指定的"不关心"的目录/文件名。路径里**任何一层**的名字(不止顶层)命中就整条忽略,
/// 例如 `packages/foo/node_modules/x/index.js`、每个子包各自的 `target/`。
///
/// `.git` 不需要写进来:`root/.git` 里引用类文件是特殊放行的,嵌套层(子模块、内嵌仓库)的
/// `.git` 恒被忽略。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IgnoreRules {
    names: Vec<String>,
}

impl IgnoreRules {
    pub fn new<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            names: names.into_iter().map(Into::into).collect(),
        }
    }

    fn matches(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }
}

/// [`watch`] 的参数。`debounce` 必填,忽略规则用 [`WatchOptions::ignore`] 设置;
/// 结构体 `non_exhaustive`,以后加选项不破坏调用方。
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WatchOptions {
    /// 安静期:连续事件的间隔小于它就算同一批,最后一个事件之后再静默这么久才回调一次。
    pub debounce: Duration,
    pub ignore: IgnoreRules,
}

impl WatchOptions {
    pub fn new(debounce: Duration) -> Self {
        Self {
            debounce,
            ignore: IgnoreRules::default(),
        }
    }

    pub fn ignore(mut self, rules: IgnoreRules) -> Self {
        self.ignore = rules;
        self
    }
}

/// 一个 debounce 窗口内的变更汇总。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitChange {
    /// 有 `.git/HEAD`、`index`、`packed-refs` 或 `refs/*` 的变化(分支切换、外部提交、
    /// 其他 worktree 提交都会碰到这几个文件)。
    pub refs_changed: bool,
    /// 有被忽略规则放行的工作区文件变化。
    pub workdir_changed: bool,
    /// 这一批里所有相关的变更路径:绝对、规范化、去重、按字典序排序;工作区路径与上面
    /// 那些 `.git` 引用文件的路径都在里面。
    pub paths: Vec<PathBuf>,
}

/// 存活的监听器。Drop 即停止:之后不会再有回调(已在途的那一批也被丢弃)。
pub struct WatchHandle {
    _watcher: notify::RecommendedWatcher,
    stopped: Arc<AtomicBool>,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

/// 监听 `root` 目录树。`on_change` 在 debounce 线程上被调用,不得长时间阻塞。
///
/// `root` 会先规范化(macOS 上 `/var/...` 与 FSEvents 回报的 `/private/var/...` 要对齐)。
/// 监听器启动失败(如文件描述符耗尽、路径不存在)返回 `GitErrorKind::Io`。
pub fn watch(
    root: &Path,
    opts: WatchOptions,
    on_change: impl FnMut(GitChange) + Send + 'static,
) -> Result<WatchHandle, GitError> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let (tx, rx) = mpsc::channel::<Batch>();
    let filter_root = root.clone();
    let ignore = opts.ignore;

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        let batch = batch_of(&filter_root, &event.paths, &ignore);
        if !batch.is_empty() {
            let _ = tx.send(batch);
        }
    })?;
    watcher.watch(&root, notify::RecursiveMode::Recursive)?;

    let stopped = Arc::new(AtomicBool::new(false));
    let thread_stopped = stopped.clone();
    let debounce = opts.debounce;
    std::thread::Builder::new()
        .name("bytegit-watch".into())
        .spawn(move || debounce_loop(rx, debounce, &thread_stopped, on_change))
        .map_err(|e| GitError::new(GitErrorKind::Io, format!("启动监听线程失败: {e}")))?;

    Ok(WatchHandle {
        _watcher: watcher,
        stopped,
    })
}

/// 一次路径变化相对这个仓库的相关性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Workdir,
    GitRefs,
}

/// `changed` 是否值得上报,值得的话是哪一类。`root/.git` 下只有 `HEAD`、`index`、
/// `packed-refs`、`refs/*` 算引用类,其余(`objects/` 等)忽略;嵌套层的 `.git`
/// (子模块、内嵌仓库)与调用方指定的名字在任何一层命中都忽略。
fn classify(root: &Path, changed: &Path, ignore: &IgnoreRules) -> Option<Kind> {
    let rel = changed.strip_prefix(root).ok()?;
    let mut parts = rel.components();
    let Some(Component::Normal(first)) = parts.next() else {
        return Some(Kind::Workdir); // 仓库根自身的事件,极少见,当作相关
    };
    let first = first.to_string_lossy();
    if first == ".git" {
        let rest: PathBuf = parts.collect();
        let rest_str = rest.to_string_lossy();
        if rest_str == "HEAD" || rest_str == "index" || rest_str == "packed-refs" {
            return Some(Kind::GitRefs);
        }
        if rest.starts_with("refs") {
            return Some(Kind::GitRefs);
        }
        return None;
    }
    if ignore.matches(&first) {
        return None;
    }
    let nested_ignored = parts.any(|c| match c {
        Component::Normal(name) => is_ignored_name(name, ignore),
        _ => false,
    });
    if nested_ignored {
        return None;
    }
    Some(Kind::Workdir)
}

fn is_ignored_name(name: &OsStr, ignore: &IgnoreRules) -> bool {
    let name = name.to_string_lossy();
    name == ".git" || ignore.matches(&name)
}

#[derive(Debug, Default)]
struct Batch {
    refs_changed: bool,
    workdir_changed: bool,
    paths: BTreeSet<PathBuf>,
}

impl Batch {
    fn is_empty(&self) -> bool {
        !self.refs_changed && !self.workdir_changed
    }

    fn merge(&mut self, other: Batch) {
        self.refs_changed |= other.refs_changed;
        self.workdir_changed |= other.workdir_changed;
        self.paths.extend(other.paths);
    }

    fn into_change(self) -> GitChange {
        GitChange {
            refs_changed: self.refs_changed,
            workdir_changed: self.workdir_changed,
            paths: self.paths.into_iter().collect(),
        }
    }
}

/// 把一个 notify 事件的路径折成一个 [`Batch`]。逐路径规范化(规范化失败,如文件刚被删,
/// 就用原路径)后再判断相关性,避免 macOS 上 `/var/...` 与 `/private/var/...` 不一致让整批
/// 事件被误判为无关。
fn batch_of(root: &Path, event_paths: &[PathBuf], ignore: &IgnoreRules) -> Batch {
    let mut batch = Batch::default();
    for path in event_paths {
        let normalized = path.canonicalize().unwrap_or_else(|_| path.clone());
        match classify(root, &normalized, ignore) {
            Some(Kind::GitRefs) => {
                batch.refs_changed = true;
                batch.paths.insert(normalized);
            }
            Some(Kind::Workdir) => {
                batch.workdir_changed = true;
                batch.paths.insert(normalized);
            }
            None => {}
        }
    }
    batch
}

/// 安静期 debounce:收到第一批后,只要下一批在 `debounce` 内到达就并进来,
/// 静默满 `debounce`(或通道关闭)就回调一次。`stopped` 为真时丢弃这一批。
fn debounce_loop(
    rx: Receiver<Batch>,
    debounce: Duration,
    stopped: &AtomicBool,
    mut on_change: impl FnMut(GitChange),
) {
    while let Ok(first) = rx.recv() {
        let mut acc = first;
        // 超时与通道关闭都结束这一批:前者是安静期满,后者是监听器已销毁。
        while let Ok(more) = rx.recv_timeout(debounce) {
            acc.merge(more);
        }
        if stopped.load(Ordering::SeqCst) {
            return;
        }
        on_change(acc.into_change());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    /// 与 dozer 的 `project::HIDDEN` 同一份名单。
    fn hidden() -> IgnoreRules {
        IgnoreRules::new([".git", "target", "node_modules", ".DS_Store"])
    }

    fn kind(changed: &str) -> Option<Kind> {
        classify(Path::new("/r"), Path::new(changed), &hidden())
    }

    // ---- classify:从 dozer `git_watch.rs` 原样迁来的断言 ----

    #[test]
    fn workdir_file_is_relevant() {
        assert_eq!(kind("/r/src/main.rs"), Some(Kind::Workdir));
    }

    #[test]
    fn hidden_dirs_are_not_relevant() {
        assert_eq!(kind("/r/target/debug/foo"), None);
        assert_eq!(kind("/r/node_modules/x/index.js"), None);
        assert_eq!(kind("/r/.DS_Store"), None);
    }

    #[test]
    fn nested_hidden_dirs_are_not_relevant() {
        // monorepo/多包项目:嵌套在子目录里的 node_modules/target/.git(子模块)同样不该触发刷新,
        // 不止顶层——否则 `npm install`/编译产物写入子包目录会被误判成"工作区改动"。
        assert_eq!(kind("/r/packages/foo/node_modules/x/index.js"), None);
        assert_eq!(kind("/r/crates/bar/target/debug/foo"), None);
        assert_eq!(kind("/r/vendor/sub/.git/HEAD"), None);
        // 但子目录本身的正常源码改动依然相关。
        assert_eq!(kind("/r/packages/foo/src/main.rs"), Some(Kind::Workdir));
    }

    #[test]
    fn git_control_files_are_relevant_as_git_refs() {
        assert_eq!(kind("/r/.git/HEAD"), Some(Kind::GitRefs));
        assert_eq!(kind("/r/.git/index"), Some(Kind::GitRefs));
        assert_eq!(kind("/r/.git/refs/heads/main"), Some(Kind::GitRefs));
        assert_eq!(kind("/r/.git/packed-refs"), Some(Kind::GitRefs));
    }

    #[test]
    fn other_git_internals_are_not_relevant() {
        assert_eq!(kind("/r/.git/objects/ab/cdef"), None);
    }

    // ---- classify:迁移前就有、此前没有测试的口径 ----

    #[test]
    fn the_repo_root_itself_is_relevant_as_workdir() {
        assert_eq!(kind("/r"), Some(Kind::Workdir));
    }

    #[test]
    fn paths_outside_the_repo_are_not_relevant() {
        assert_eq!(kind("/elsewhere/src/main.rs"), None);
        assert_eq!(kind("/rr/src/main.rs"), None, "前缀相同但不是子路径");
    }

    #[test]
    fn git_lock_files_and_lookalikes_are_not_refs_except_under_refs() {
        assert_eq!(kind("/r/.git/index.lock"), None);
        assert_eq!(kind("/r/.git/HEAD.lock"), None);
        assert_eq!(kind("/r/.git/refsx"), None, "按路径分量判断,不是字符串前缀");
        // `refs/` 之下一律算引用类,包括 git 写引用时的临时 `.lock` 文件。
        assert_eq!(kind("/r/.git/refs/heads/main.lock"), Some(Kind::GitRefs));
        assert_eq!(kind("/r/.git/refs"), Some(Kind::GitRefs));
    }

    #[test]
    fn dotfiles_outside_the_ignore_list_are_relevant() {
        assert_eq!(kind("/r/.env"), Some(Kind::Workdir));
        assert_eq!(kind("/r/.gitignore"), Some(Kind::Workdir));
        assert_eq!(kind("/r/src/.hidden/x"), Some(Kind::Workdir));
    }

    #[test]
    fn a_dot_git_file_at_the_root_is_not_relevant() {
        // linked worktree / 子模块里 `.git` 是个指向别处 gitdir 的文件:这个文件本身的变化不上报
        // (真实 gitdir 在 root 之外,不在监听范围内——见模块文档的已知限制)。
        assert_eq!(kind("/r/.git"), None);
    }

    #[test]
    fn ignore_rules_are_defined_by_the_caller() {
        let none = IgnoreRules::default();
        let c = |p: &str| classify(Path::new("/r"), Path::new(p), &none);
        // 没有规则时,`target`/`node_modules` 都算工作区变化。
        assert_eq!(c("/r/target/debug/foo"), Some(Kind::Workdir));
        assert_eq!(c("/r/a/node_modules/x"), Some(Kind::Workdir));
        // 但 `.git` 的特殊处理是内置的,不依赖规则:根下按引用文件判断,嵌套层恒忽略。
        assert_eq!(c("/r/.git/HEAD"), Some(Kind::GitRefs));
        assert_eq!(c("/r/.git/objects/ab"), None);
        assert_eq!(c("/r/vendor/sub/.git/HEAD"), None);
        let only_dist = IgnoreRules::new(["dist"]);
        assert_eq!(
            classify(Path::new("/r"), Path::new("/r/a/dist/x.js"), &only_dist),
            None
        );
    }

    // ---- batch_of ----

    #[test]
    fn a_batch_flags_refs_and_workdir_independently_and_keeps_every_relevant_path() {
        let paths = vec![
            PathBuf::from("/r/.git/HEAD"),
            PathBuf::from("/r/src/a.rs"),
            PathBuf::from("/r/target/x"),
            PathBuf::from("/r/src/a.rs"),
        ];
        let b = batch_of(Path::new("/r"), &paths, &hidden());
        assert!(b.refs_changed && b.workdir_changed);
        assert_eq!(
            b.paths.into_iter().collect::<Vec<_>>(),
            vec![PathBuf::from("/r/.git/HEAD"), PathBuf::from("/r/src/a.rs")]
        );
    }

    /// macOS 上 `/var/...` 是 `/private/var/...` 的符号链接,FSEvents 可能用任一形式回报路径;
    /// 不先规范化,`strip_prefix(root)` 会失败、整批事件被误判为无关。
    #[cfg(unix)]
    #[test]
    fn event_paths_are_canonicalized_through_symlinks_before_classifying() {
        let real = tempfile::tempdir().unwrap();
        let root = real.path().canonicalize().unwrap();
        std::fs::write(root.join("a.rs"), "x").unwrap();
        let links = tempfile::tempdir().unwrap();
        let link = links.path().join("link");
        std::os::unix::fs::symlink(&root, &link).unwrap();

        let b = batch_of(&root, &[link.join("a.rs")], &hidden());
        assert!(b.workdir_changed);
        assert_eq!(
            b.paths.into_iter().collect::<Vec<_>>(),
            vec![root.join("a.rs")],
            "上报的是规范化后的路径"
        );
    }

    #[test]
    fn a_batch_with_nothing_relevant_is_empty() {
        let paths = vec![
            PathBuf::from("/r/target/x"),
            PathBuf::from("/r/.git/objects/ab"),
        ];
        assert!(batch_of(Path::new("/r"), &paths, &hidden()).is_empty());
    }

    // ---- debounce_loop:不碰文件系统的确定性测试 ----

    fn workdir(path: &str) -> Batch {
        Batch {
            workdir_changed: true,
            paths: BTreeSet::from([PathBuf::from(path)]),
            ..Batch::default()
        }
    }

    fn refs(path: &str) -> Batch {
        Batch {
            refs_changed: true,
            paths: BTreeSet::from([PathBuf::from(path)]),
            ..Batch::default()
        }
    }

    /// 在一个线程里跑 `debounce_loop`,返回它回调出的所有 `GitChange`。
    fn run_loop(
        debounce: Duration,
        stopped: bool,
        feed: impl FnOnce(&mpsc::Sender<Batch>),
    ) -> Vec<GitChange> {
        let (tx, rx) = mpsc::channel();
        let out = Arc::new(Mutex::new(Vec::new()));
        let out2 = out.clone();
        let stopped = Arc::new(AtomicBool::new(stopped));
        let handle = std::thread::spawn(move || {
            debounce_loop(rx, debounce, &stopped, move |c| {
                out2.lock().unwrap().push(c)
            });
        });
        feed(&tx);
        drop(tx);
        handle.join().unwrap();
        Arc::try_unwrap(out)
            .expect("线程已结束,不再有其他持有者")
            .into_inner()
            .unwrap()
    }

    #[test]
    fn batches_arriving_within_the_window_merge_into_one_change() {
        let got = run_loop(Duration::from_secs(5), false, |tx| {
            tx.send(workdir("/r/b.rs")).unwrap();
            tx.send(workdir("/r/a.rs")).unwrap();
            tx.send(workdir("/r/a.rs")).unwrap();
        });
        assert_eq!(got.len(), 1);
        assert!(got[0].workdir_changed && !got[0].refs_changed);
        assert_eq!(
            got[0].paths,
            vec![PathBuf::from("/r/a.rs"), PathBuf::from("/r/b.rs")],
            "去重并排序"
        );
    }

    #[test]
    fn a_refs_change_anywhere_in_the_window_sets_refs_changed() {
        let got = run_loop(Duration::from_secs(5), false, |tx| {
            tx.send(workdir("/r/a.rs")).unwrap();
            tx.send(refs("/r/.git/HEAD")).unwrap();
            tx.send(workdir("/r/b.rs")).unwrap();
        });
        assert_eq!(got.len(), 1);
        assert!(got[0].refs_changed && got[0].workdir_changed);
        assert_eq!(got[0].paths.len(), 3);
    }

    #[test]
    fn a_quiet_period_splits_changes_into_separate_callbacks() {
        let got = run_loop(Duration::from_millis(60), false, |tx| {
            tx.send(workdir("/r/a.rs")).unwrap();
            std::thread::sleep(Duration::from_millis(400));
            tx.send(refs("/r/.git/HEAD")).unwrap();
        });
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got[0].workdir_changed && !got[0].refs_changed);
        assert!(got[1].refs_changed && !got[1].workdir_changed);
    }

    #[test]
    fn nothing_is_delivered_once_the_handle_is_stopped() {
        let got = run_loop(Duration::from_millis(10), true, |tx| {
            tx.send(workdir("/r/a.rs")).unwrap();
        });
        assert!(got.is_empty());
    }

    // ---- watch:真实文件系统(macOS FSEvents 在某些沙盒/CI 环境不向临时目录投递事件,
    //      这时回调数为 0,上面的确定性测试已覆盖算法本身,这里不把平台能力缺失误报成失败) ----

    fn wait_for(count: &AtomicUsize, at_least: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while count.load(Ordering::SeqCst) < at_least && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn counting(count: &Arc<AtomicUsize>) -> impl FnMut(GitChange) + Send + 'static {
        let count = count.clone();
        move |_| {
            count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn watch_debounces_rapid_writes_into_one_callback() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        std::fs::create_dir_all(repo.join(".git")).unwrap(); // 贴近真实仓库
        for i in 0..5 {
            std::fs::write(repo.join(format!("f{i}.txt")), "initial").unwrap();
        }
        let count = Arc::new(AtomicUsize::new(0));
        let _handle = watch(
            &repo,
            WatchOptions::new(Duration::from_millis(100)).ignore(hidden()),
            counting(&count),
        )
        .expect("watcher 应能启动");

        for i in 0..5 {
            std::fs::write(repo.join(format!("f{i}.txt")), "x").unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        // FSEvents 可能把临时目录的事件延迟数百毫秒才投递:先等首个回调(上限 5s),
        // 再多等一个 debounce 窗口,避免把后端通知延迟误判成 debounce 失效。
        wait_for(&count, 1, Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(350));

        let callbacks = count.load(Ordering::SeqCst);
        if callbacks == 0 {
            return;
        }
        assert_eq!(callbacks, 1, "5 次快速写入应合并成 1 次回调");
    }

    #[test]
    fn watch_ignores_writes_under_ignored_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        std::fs::create_dir_all(repo.join("target")).unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let _handle = watch(
            &repo,
            WatchOptions::new(Duration::from_millis(100)).ignore(hidden()),
            counting(&count),
        )
        .unwrap();

        std::fs::write(repo.join("target").join("build-artifact"), "x").unwrap();
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "target/ 下的改动不该触发回调"
        );
    }

    #[test]
    fn watch_reports_a_git_ref_change_as_refs_changed() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let seen = Arc::new(Mutex::new(Vec::<GitChange>::new()));
        let seen2 = seen.clone();
        let _handle = watch(
            &repo,
            WatchOptions::new(Duration::from_millis(100)).ignore(hidden()),
            move |c| seen2.lock().unwrap().push(c),
        )
        .unwrap();

        std::fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/other\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while seen.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let seen = seen.lock().unwrap();
        if seen.is_empty() {
            return;
        }
        assert!(seen.iter().any(|c| c.refs_changed), "{seen:?}");
    }

    #[test]
    fn no_callbacks_after_the_handle_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let count = Arc::new(AtomicUsize::new(0));
        let handle = watch(
            &repo,
            WatchOptions::new(Duration::from_millis(50)),
            counting(&count),
        )
        .unwrap();
        std::fs::write(repo.join("a.txt"), "1").unwrap();
        drop(handle);
        std::thread::sleep(Duration::from_millis(100));
        let at_drop = count.load(Ordering::SeqCst);
        std::fs::write(repo.join("a.txt"), "2").unwrap();
        std::fs::write(repo.join("b.txt"), "3").unwrap();
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(
            count.load(Ordering::SeqCst),
            at_drop,
            "Drop 之后不能再有回调"
        );
    }

    #[test]
    fn watching_a_missing_path_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = watch(
            &dir.path().join("does-not-exist"),
            WatchOptions::new(Duration::from_millis(50)),
            |_| {},
        )
        .err()
        .expect("不存在的路径应启动失败");
        assert_eq!(err.kind(), GitErrorKind::Io);
    }
}
```

- [ ] **Step 4: 在 `src/lib.rs` 声明并再导出**

在 `pub mod testutil;` 之后加 `#[cfg(feature = "watch")]\nmod watch;`;在文件末尾加 `#[cfg(feature = "watch")]\npub use watch::{GitChange, IgnoreRules, WatchHandle, WatchOptions, watch};`。在 `v0.3.0`(P3 尚未合入)的基础上,`src/lib.rs` 完整为:

```rust
//! bytegit:ByteBoy 系产品共用的本地 Git 底层。
//!
//! 全部同步;公开 API 不暴露 `git2` 类型。设计见 dozer 仓库
//! `docs/superpowers/specs/2026-10-02-bytegit-design.md`。

mod change;
mod content;
mod error;
mod history;
mod id;
mod info;
mod repo;
mod status;
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;
#[cfg(feature = "watch")]
mod watch;

pub use change::ChangeKind;
pub use content::{Content, ContentLimits, ContentPair};
pub use error::{GitError, GitErrorKind};
pub use history::{CommitSummary, FileChange, LogOptions, Patch, Signature};
pub use id::{BlobId, CommitId};
pub use info::{HeadInfo, Remote};
pub use repo::Repo;
pub use status::{FileState, StatusEntry, StatusOptions};
#[cfg(feature = "watch")]
pub use watch::{GitChange, IgnoreRules, WatchHandle, WatchOptions, watch};
```

(若 P3 已先合入,`mod stats;` 等它自己的行原样保留,只加上面这两处。)

- [ ] **Step 5: 运行测试与门禁**

Run: `cargo test --features watch watch:: && cargo test && cargo test --all-features`
Expected: `watch::` 23 个通过;不开 feature 时全量 84 个(`watch` 模块与 `notify` 都不参与);`--all-features` 全量 107 个。

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings`
Expected: 干净。

Run: `cargo tree -e normal | grep -c notify`
Expected: `0`(不开 feature 时依赖树里没有 `notify`)。

- [ ] **Step 6: 变异检验(确认 Review Focus 的测试真的会失败)**

先 `git add Cargo.toml src/watch.rs src/error.rs src/lib.rs` 暂存当前版本(`watch.rs` 是新文件,不暂存的话没法还原);逐个做、逐个用 `git checkout -- src/watch.rs` 还原到暂存版本:

1. 删掉 `classify` 里的 `if rest.starts_with("refs") { return Some(Kind::GitRefs); }` 三行。Run: `cargo test --features watch watch::`。Expected: `git_control_files_are_relevant_as_git_refs`、`git_lock_files_and_lookalikes_are_not_refs_except_under_refs` 失败。
2. 删掉 `classify` 里的 `if nested_ignored { return None; }` 三行。Expected: `nested_hidden_dirs_are_not_relevant`、`ignore_rules_are_defined_by_the_caller` 失败。
3. 把 `debounce_loop` 里的 `acc.merge(more);` 改成空语句(`let _ = more;`)。Expected: `batches_arriving_within_the_window_merge_into_one_change`、`a_refs_change_anywhere_in_the_window_sets_refs_changed` 失败。
4. 删掉 `debounce_loop` 里的 `if stopped.load(Ordering::SeqCst) { return; }` 三行。Expected: `nothing_is_delivered_once_the_handle_is_stopped` 失败。
5. 把 `is_ignored_name` 里的 `name == ".git" || ignore.matches(&name)` 改成 `ignore.matches(&name)`。Expected: `ignore_rules_are_defined_by_the_caller` 失败。
6. 把 `batch_of` 里 `Some(Kind::GitRefs)` 分支的 `batch.refs_changed = true;` 改成 `batch.workdir_changed = true;`。Expected: `a_batch_flags_refs_and_workdir_independently_and_keeps_every_relevant_path`、`watch_reports_a_git_ref_change_as_refs_changed` 失败。
7. 把 `batch_of` 里的 `let normalized = path.canonicalize().unwrap_or_else(|_| path.clone());` 改成 `let normalized = path.clone();`。Expected: `event_paths_are_canonicalized_through_symlinks_before_classifying` 失败。

- [ ] **Step 7: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git status --short
git add Cargo.toml src/watch.rs src/error.rs src/lib.rs
git diff --cached --stat
git commit -m "feat: add watch feature (debounced fs change watching)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: bytegit — 发布 v0.4.0

**Files:**
- Modify: `Cargo.toml`(`version = "0.4.0"`)

**Interfaces:** Produces:git tag `v0.4.0`(Task 4 的 dozer 依赖)。

- [ ] **Step 1: 升版本并全量门禁**

把 `Cargo.toml` 的 `version = "0.3.0"` 改为 `"0.4.0"`(版本号顺延规则见"前置条件")。

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings && cargo test && cargo test --all-features`
Expected: 全部通过,默认 84 个、`--all-features` 107 个。

- [ ] **Step 2: Commit 并推送 main(对外可见操作:先向用户确认)**

向用户确认"现在推送 `main` 并打 tag `v0.4.0`"。获得明确同意后:

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git add Cargo.toml
git commit -m "chore: release 0.4.0

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push origin main
gh run watch "$(gh run list --repo byteboyai/bytegit --limit 1 --json databaseId -q '.[0].databaseId')" --repo byteboyai/bytegit --exit-status
```

Expected: CI(macOS,已含 `cargo clippy --all-features` 与 `cargo test --all-features`,无需改 `ci.yml`)通过。**CI 红则先修,不打 tag。**(macOS runner 上端到端测试若收不到 FSEvents,会按设计静默跳过,不会让 CI 变红。)

- [ ] **Step 3: 打 tag 并验证可被消费**

```bash
git tag -a v0.4.0 -m "bytegit v0.4.0: watch feature"
git push origin v0.4.0
cd "$(mktemp -d)" && cargo new --lib consume -q && cd consume
cargo add bytegit --git https://github.com/byteboyai/bytegit --tag v0.4.0 --features watch
cat >> src/lib.rs <<'EOF'

#[cfg(test)]
mod t {
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn consumes_bytegit_watch() {
        let dir = std::env::temp_dir().join(format!("bytegit-consume-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = mpsc::channel();
        let _handle = bytegit::watch(
            &dir,
            bytegit::WatchOptions::new(Duration::from_millis(50))
                .ignore(bytegit::IgnoreRules::new(["target"])),
            move |change| {
                let _ = tx.send(change);
            },
        )
        .unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        // 能启动即可;收得到事件时再断言内容(某些沙盒环境不投递 FSEvents)。
        if let Ok(change) = rx.recv_timeout(Duration::from_secs(5)) {
            assert!(change.workdir_changed);
        }
    }
}
EOF
cargo test
```

Expected: `consumes_bytegit_watch ... ok`。

---

### Task 3: dozer — 在旧 `git_watch.rs` 上补刻画测试

**Files:**
- Modify(worktree 内,只改 `tests` 模块): `crates/dozer-app/src/git_watch.rs`

**Interfaces:** 无新接口。产出 6 个刻画测试(5 个纯函数 + 1 个端到端),在**旧实现**上全部通过;Task 4 删除 `git_watch.rs` 时这些断言已经在 bytegit 里以同样的期望保留(Task 1 的 `classify` 测试)。

- [ ] **Step 1: 建专用 worktree 与分支**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytegit-p4 -b bytegit-p4 main
mkdir -p ../dozer-bytegit-p4/.cargo && cp .cargo/config.toml ../dozer-bytegit-p4/.cargo/config.toml
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p4 && git log --oneline | head -1 && git status --short
```

Expected: 干净、分支 `bytegit-p4`,HEAD 与 `main` 一致。`.cargo/config.toml`(不提交,已被 `.gitignore`)里的 `[patch."https://github.com/byteboyai/bytegit"]` 指向本地 `../bytegit`,此时它已含 Task 1–2 的改动。此后**所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytegit-p4/` 前缀**。

- [ ] **Step 2: 给 `git_watch.rs` 的 `tests` 模块加 6 个刻画测试**

在 `crates/dozer-app/src/git_watch.rs` 的 `mod tests` 末尾(最后一个测试之后、模块结尾 `}` 之前)追加:

```rust
    // ---- bytegit P4:迁移前就有、此前没有测试的口径 ----

    #[test]
    fn the_repo_root_itself_is_relevant_as_workdir() {
        let repo = Path::new("/r");
        assert_eq!(
            is_relevant_path(repo, Path::new("/r")),
            Some(Relevance::Workdir)
        );
    }

    #[test]
    fn paths_outside_the_repo_are_not_relevant() {
        let repo = Path::new("/r");
        assert_eq!(
            is_relevant_path(repo, Path::new("/elsewhere/src/main.rs")),
            None
        );
        assert_eq!(
            is_relevant_path(repo, Path::new("/rr/src/main.rs")),
            None,
            "前缀相同但不是子路径"
        );
    }

    #[test]
    fn git_lock_files_and_lookalikes_are_not_refs_except_under_refs() {
        let repo = Path::new("/r");
        let rel = |p: &str| is_relevant_path(repo, Path::new(p));
        assert_eq!(rel("/r/.git/index.lock"), None);
        assert_eq!(rel("/r/.git/HEAD.lock"), None);
        assert_eq!(rel("/r/.git/refsx"), None, "按路径分量判断,不是字符串前缀");
        // `refs/` 之下一律算引用类,包括 git 写引用时的临时 `.lock` 文件。
        assert_eq!(
            rel("/r/.git/refs/heads/main.lock"),
            Some(Relevance::GitRefs)
        );
        assert_eq!(rel("/r/.git/refs"), Some(Relevance::GitRefs));
    }

    #[test]
    fn dotfiles_outside_the_hidden_list_are_relevant() {
        let repo = Path::new("/r");
        let rel = |p: &str| is_relevant_path(repo, Path::new(p));
        assert_eq!(rel("/r/.env"), Some(Relevance::Workdir));
        assert_eq!(rel("/r/.gitignore"), Some(Relevance::Workdir));
        assert_eq!(rel("/r/src/.hidden/x"), Some(Relevance::Workdir));
    }

    #[test]
    fn a_dot_git_file_at_the_root_is_not_relevant() {
        // linked worktree / 子模块里 `.git` 是个指向别处 gitdir 的文件:这个文件本身的变化不上报
        // (真实 gitdir 在仓库根之外,从来不在监听范围内——规格 O3 的已知缺口)。
        let repo = Path::new("/r");
        assert_eq!(is_relevant_path(repo, Path::new("/r/.git")), None);
    }

    #[test]
    fn start_reports_a_git_ref_change_as_git_refs() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<FsChanges>::new()));
        let seen2 = seen.clone();
        let _handle = rt.block_on(async {
            start(
                &tokio::runtime::Handle::current(),
                repo.clone(),
                Duration::from_millis(100),
                move |changes| seen2.lock().unwrap().push(changes),
            )
            .unwrap()
        });

        std::fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/other\n").unwrap();
        rt.block_on(async {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while seen.lock().unwrap().is_empty() && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let seen = seen.lock().unwrap();
        if seen.is_empty() {
            // 某些沙盒/CI 环境不向临时目录投递 FSEvents;分类算法由上面的纯函数测试覆盖。
            return;
        }
        assert!(
            seen.iter().any(|c| c.relevance == Some(Relevance::GitRefs)),
            "{seen:?}"
        );
    }
```

- [ ] **Step 3: 在旧实现上运行,确认全部通过**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p4 && cargo fmt -p dozer-app && git status --short`
Expected: 只有 `crates/dozer-app/src/git_watch.rs` 变了。

Run: `cargo test -p dozer-app git_watch`
Expected: 13 个全部通过(原有 7 个 + 新增 6 个)。**任何新增测试在旧实现上失败,说明我对旧行为的描述有误——先停下确认旧行为,不要改实现,也不要直接改断言迎合。**(端到端那条在不投递 FSEvents 的环境里会静默通过,分类算法由其余 5 条纯函数测试覆盖。)

- [ ] **Step 4: 门禁与 Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p4
git add crates/dozer-app/src/git_watch.rs
git diff --cached --stat
git commit -m "test(git-watch): characterize path classification and ref-change reporting before bytegit P4 migration

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: dozer — 切换到 `bytegit::watch`,删除 `git_watch.rs`

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`、`Cargo.lock`、`crates/dozer-app/src/main.rs`、`crates/dozer-app/src/app/message.rs`、`crates/dozer-app/src/app/update.rs`、`crates/dozer-app/src/workspace/state.rs`
- Delete: `crates/dozer-app/src/git_watch.rs`

**Interfaces:**
- Consumes: bytegit `v0.4.0`(feature `watch`)的 `watch`、`WatchOptions`、`IgnoreRules`、`GitChange`、`WatchHandle`;`project::HIDDEN: [&str; 4]`
- Produces: `Message::ProjectFsChanged(ProjectId, bytegit::GitChange)`;`Workspace.git_watch: Option<bytegit::WatchHandle>`;`App::project_fs_changed(&mut self, ProjectId, bytegit::GitChange)`

- [ ] **Step 1: 升依赖、开 feature、去掉 `notify`、让 `Cargo.lock` 指向 tag**

1. `crates/dozer-app/Cargo.toml`:`bytegit = { ... tag = "v0.3.0" }` 改成 `bytegit = { git = "https://github.com/byteboyai/bytegit", tag = "v0.4.0", features = ["watch"] }`;删掉 `# notify = 项目工作区实时文件系统监听(git_watch 的 debounce 基础)。` 那行注释与 `notify = "8"` 那行(`dozer-app` 里除 `git_watch.rs` 外没有别处用 `notify`)。
2. 让 `Cargo.lock` 指向真实 tag(而不是本地 patch):临时把 `.cargo/config.toml` 里 `[patch."https://github.com/byteboyai/bytegit"]` 与下一行注释掉(保留 byteui 的 patch),运行 `cargo update -p bytegit`,再 `git diff Cargo.lock` 确认**只有** `bytegit` 的 version 与 `source` 的 tag 变化,以及 `dozer-app` 的依赖列表里少了 `notify`(`notify` 包本身仍在锁文件里,因为 bytegit 依赖它);`source = "git+..."` 行都在。确认后恢复 `.cargo/config.toml`(之后若 `cargo build` 又去掉了 `source` 行,提交前 `git checkout Cargo.lock` 还原再重复本步)。

- [ ] **Step 2: 切换代码**

1. `crates/dozer-app/src/main.rs`:删掉 `mod git_watch;`。
2. `crates/dozer-app/src/app/message.rs`:
   - 删掉 `use crate::git_watch;`;
   - `ProjectFsChanged` 的文档注释里,把"`git_watch` 监听到工作区/`.git` 引用变化,该重新跑一次 git 刷新\n    /// 了(D4)。`Relevance` 决定这次触发要不要顺带做 Plan 2 的 Git Log 快照\n    /// 重建。"改为"`bytegit::watch` 监听到工作区/`.git` 引用变化,该重新跑一次 git 刷新\n    /// 了(D4)。`GitChange::refs_changed` 决定这次触发要不要顺带做 Plan 2 的\n    /// Git Log 快照重建。"(后面"这条消息同时喂给 Files……"的部分不动);
   - `ProjectFsChanged(ProjectId, git_watch::FsChanges),` → `ProjectFsChanged(ProjectId, bytegit::GitChange),`。
3. `crates/dozer-app/src/app/update.rs`:
   - 删掉 `use crate::git_watch;`;
   - `project_fs_changed` 的参数 `changes: git_watch::FsChanges,` → `changes: bytegit::GitChange,`;
   - 把条件

     ```rust
     if changes.relevance == Some(git_watch::Relevance::Workdir)
         || changes.relevance == Some(git_watch::Relevance::GitRefs)
     {
     ```

     改为 `if changes.workdir_changed || changes.refs_changed {`;
   - 把 `if changes.relevance == Some(git_watch::Relevance::GitRefs)\n            && self.active_project_id == Some(project_id)` 的第一行改为 `if changes.refs_changed`(后面的 `&& self.active_project_id ...` 等不动)。
4. `crates/dozer-app/src/workspace/state.rs`:
   - 删掉 `use crate::git_watch;`;
   - 字段 `pub(crate) git_watch: Option<git_watch::Handle>,` → `pub(crate) git_watch: Option<bytegit::WatchHandle>,`;
   - 把 `start_git_watch` 里的 `match git_watch::start( ... ) {`(从 `match git_watch::start(` 到它的 `) {`)替换为:

```rust
        // 忽略名单由我们自己传:文件树恒不显示的那几个目录/文件名(含嵌套层)的改动不触发刷新。
        let options = bytegit::WatchOptions::new(std::time::Duration::from_millis(300))
            .ignore(bytegit::IgnoreRules::new(crate::project::HIDDEN));
        match bytegit::watch(&repo, options, move |changes| {
            let _ = proxy.send_event(Message::ProjectFsChanged(project_id, changes));
        }) {
```

     其余(`Ok(handle) => self.git_watch = Some(handle),` 与 `Err` 分支)不动,只把 `Err` 分支日志里的文案 `git_watch 启动失败,降级为手动刷新` 改成 `bytegit watch 启动失败,降级为手动刷新`。
5. 删除 `crates/dozer-app/src/git_watch.rs`(`git rm`)。

注释里提到 `git_watch` 的其余几处(`files/state.rs`、`files/update.rs`、`git_log.rs`、`workspace/state.rs` 的字段名与 `start_git_watch` 方法名)指代的是"监听 → `ProjectFsChanged`"这套机制,不改。

- [ ] **Step 3: 编译并运行**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p4 && cargo fmt -p dozer-app && git status --short && cargo build -p dozer-app 2>&1 | grep -E "^(warning|error)" -A6`
Expected: `git status` 只有本任务列出的文件;无 error;警告只有"已知基线"里与本计划无关的两条。

Run: `cargo test -p dozer-app 2>&1 | grep -E "FAILED|test result"`
Expected: 唯一失败是已知基线的 `delete_confirm_spec_reflects_pending_target`;通过数比 Task 3 之后(迁移前)**少 13 个**(迁到 bytegit 的 `git_watch` 测试)。

- [ ] **Step 4: 门禁**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "workspace/state.rs|app/update.rs|app/message.rs|src/main.rs"`
Expected: 只有 `workspace/state.rs` 里两条既有诊断(与迁移前逐条相同,行号偏移约 3 行);其余文件无输出。

Run: `grep -rn "git_watch::\|use crate::git_watch\|mod git_watch" crates/dozer-app/src`
Expected: 无输出。

- [ ] **Step 5: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p4
git status --short
git add Cargo.lock crates/dozer-app/Cargo.toml crates/dozer-app/src/main.rs crates/dozer-app/src/app/message.rs crates/dozer-app/src/app/update.rs crates/dozer-app/src/workspace/state.rs
git rm -q crates/dozer-app/src/git_watch.rs
git diff --cached --stat
git commit -m "refactor(git-watch): back project fs watching with bytegit::watch, drop git_watch.rs

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: `git diff --cached --stat` 里没有 `.cargo/config.toml`。

---

### Task 5: 文档同步与收尾

**Files:**
- Modify(worktree 内): `CLAUDE.md`、`docs/superpowers/specs/2026-10-02-bytegit-design.md`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`、`docs/dozer-v2/bytegit-调用点盘点.md`
- Modify(记忆): `~/.claude/projects/-Users-chrischiang-Projects-CoralProjects-byteboy-dozer/memory/dozer-v2-panel-independence-and-bytegit.md`

**Interfaces:** 无代码接口。

- [ ] **Step 1: `CLAUDE.md` 的 bytegit 条目更新进度**

把"已迁移/未迁移"的描述更新为:P4(`git_watch` 的路径分类、`.git` 引用文件识别、debounce)已迁移——项目工作区监听现在是 `bytegit::watch`(feature `watch`),忽略名单由 dozer 传入 `project::HIDDEN`;`dozer-app` 不再直接依赖 `notify`。**未迁移**只剩 P5 的 `init/clone/checkout` 及 `delivery.rs`/`homespace.rs` 里剩余的命令行 `git`(P3 若尚未合并,同时保留它的未迁移项)。并加一句:**`.git` 是文件的仓库(linked worktree/子模块)的引用变化目前收不到,见规格 O3。** 若条目里还没有 P1–P3 的措辞,只改与 P4 相关的部分,不要替它们写。

- [ ] **Step 2: 规格同步**

在 `docs/superpowers/specs/2026-10-02-bytegit-design.md`:

1. §4.5 与实现对齐:
   - `watch` 是 crate 根的自由函数 `bytegit::watch(root, opts, on_change)`;`WatchOptions::new(debounce).ignore(rules)` 构造(`#[non_exhaustive]`);`IgnoreRules::new(names)`;
   - `IgnoreRules`:路径里**任一层**的名字命中即忽略;**`.git` 的处理是内置的、不依赖规则**——根下 `.git` 只放行 `HEAD`/`index`/`packed-refs`/`refs/*`(其余如 `objects/`、`index.lock` 忽略),嵌套层的 `.git`(子模块、内嵌仓库)恒忽略;
   - `GitChange { refs_changed, workdir_changed, paths }`:`paths` 是这一批里**所有**相关路径(工作区路径与 `.git` 引用文件路径),绝对、规范化、去重、按字典序排序;
   - debounce 是"安静期"语义(最后一个事件之后静默满 `debounce` 才回调一次);`on_change` 运行在 bytegit 自己的标准线程上;`WatchHandle` Drop 之后不再有回调(含在途的一批);
   - 启动失败返回 `GitErrorKind::Io`;feature `watch` 引入 `notify 8`,不依赖 tokio;
   - "`.git` 是文件"的缺口原文保留,补一句 P4 的实测:现状只监听 `root` 目录树,linked worktree 的真实 gitdir 在树外,不被监听。
2. §6 P4 一行:验收补充"分类断言在旧/新实现上都通过(刻画测试)";"删除的旧代码"注明是整个 `git_watch.rs`。
3. §8:O3 补一句 P4 的结论(同上,P4 未实现,待用户决定是否排期);加 **O15**(= 本计划末尾"规格补充"那条):`watch` 不过滤 notify 的事件种类,Linux inotify 的 Access 事件可能让 dozer 读文件反过来触发刷新,非 macOS 发布前处理。

- [ ] **Step 3: 要求文档、盘点文档、记忆**

- `docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` §7 第 1 条后追加:`bytegit` P4 已完成(`v0.4.0`),项目工作区监听已进 `bytegit`,Host 只负责把 `GitChange` 转成消息。
- `docs/dozer-v2/bytegit-调用点盘点.md`:在 §1 末尾加一行"P4 之后,`git_watch` 的路径分类与 debounce 已合并进 bytegit,`dozer-app` 不再直接依赖 `notify`";并 `grep -n "git_watch\|notify" docs/dozer-v2/bytegit-调用点盘点.md`,把命中的调用点标注为"P4 已迁移"。
- 记忆文件追加 P4 结果与待决事项 D8/D9/D10(一两行),并更新分支(`bytegit-p4`)的合并现状。

- [ ] **Step 4: Commit(worktree)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p4
git status --short
git add CLAUDE.md docs/superpowers/specs/2026-10-02-bytegit-design.md docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md docs/dozer-v2/bytegit-调用点盘点.md
git diff --cached --stat
git commit -m "docs(bytegit): record P4 results, watch API semantics and open decisions

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: 交给用户合并**

不要自己合并 `bytegit-p4` 到 `main`:向用户报告分支、提交列表、"已知基线"里的失败、待决事项 D8/D9/D10,并请用户审阅后决定合并方式。合并前请用户在真实项目上手点一遍:① 外部编辑器改一个文件,文件树/预览是否在 300ms 左右跟进;② 在终端里 `git checkout` 切分支,顶栏分支名与 Git Log 面板是否刷新;③ 往 `node_modules` 或 `target` 里写文件,界面不应有任何刷新;④ 切换项目页签后旧项目的改动不应再触发刷新。**这些接线(`Message::ProjectFsChanged` → 各面板刷新)没有自动化测试覆盖,只靠编译与人工验收。**

---

## Self-Review

**Spec coverage(规格 §6 的 P4 行与 §4.5):**
- `watch(root, opts, on_change)`、`WatchOptions { debounce, ignore }`、`GitChange { refs_changed, workdir_changed, paths }`、`WatchHandle` Drop 即停止:Task 1;`notify` + 标准线程、不依赖 tokio:Task 1 Step 5 用 `cargo tree` 与 clippy 验证。
- `.git` 下只对 `HEAD`/`index`/`packed-refs`/`refs/*` 视为引用变化(沿用现有规则):`classify` + Task 1 测试;`IgnoreRules` 由调用方传入、含嵌套层:Task 1 测试 + Task 4 传入 `project::HIDDEN`。
- 迁移 `git_watch`、删除路径分类逻辑:Task 4(整个文件删除);现有 `git_watch` 测试迁移后通过:原有 7 个测试的断言原样进了 bytegit 的 `classify`/`watch_*` 测试,Task 3 在旧实现上补的 6 条同样保留。
- 规格"`.git` 是文件的情况":**明确不实现**(规格 O3 写"v0.1 不强求"),写进模块文档、用测试固定现状、列为 D10 由用户决定是否排期。
- 规格"事件发布不在 bytegit":符合,`on_change` 里由 dozer 自己 `proxy.send_event`。

**Placeholder scan:** 无 TBD/TODO;Task 1 给了完整文件,Task 3 给了完整测试,Task 4 对每个文件给了精确的"旧文本 → 新文本"替换。

**Type consistency:** `GitChange`/`WatchHandle`/`WatchOptions`/`IgnoreRules` 在 Task 1 定义,Task 4 的 `Message::ProjectFsChanged`、`Workspace.git_watch`、`project_fs_changed` 使用同名同形;`project::HIDDEN: [&str; 4]` 满足 `IgnoreRules::new` 的 `IntoIterator<Item: Into<String>>`。

**Review Focus:** 五条各自有对应测试(分别见各条所指任务),其中 bytegit 侧 7 个变异检验已在草稿里实际确认会让指定测试失败;dozer 侧的接线没有自动化覆盖,已在 Task 4/5 明说并给出人工验收清单。
