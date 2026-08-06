# Dozer git2 数据层迁移 + 文件树暂存态/多 worktree 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `delivery.rs` 的 git 分支/脏/文件状态查询从 shell 出 `git` CLI 迁移到 `git2`,让文件树能区分暂存/工作区改动、认识同仓库的其他 worktree,并靠 `notify` 做实时刷新(不再只在开项目/回合结束时刷新)。

**Architecture:** `delivery.rs` 内新增/改写的纯函数(`file_statuses`/`branch`/`is_dirty`/`worktrees`)全部基于 `git2::Repository`,签名尽量与旧版兼容以缩小改动面;新增 `git_watch.rs` 封装一个 debounced 的 `notify` 文件系统监听器,通过既有的 `EventLoopProxy<Message>` 把"该刷新了"这件事送回 `workspace.rs` 的消息循环,复用现有 `spawn_project_git_refresh` 管线,不新开一条刷新路径。

**Tech Stack:** Rust, `git2` 0.21(已因 `gleisbau` 成为传递依赖,本计划把它转正为 `dozer-app` 的直接依赖), `notify` 8.x(新增依赖,规格 §4 已 accepted),iced 0.14,tokio。

## Global Constraints

- 只读:本计划不新增任何会改写仓库状态的操作(不暂存、不提交、不 checkout)。
- 写路径(`accept()`/`update-ref`)维持现状 shell 出 `git` CLI,不在本计划改动范围。
- `crates/dozer-app` 是唯一涉及的 crate;不改协议、不碰 `dozerd`/`dozer-core`。
- 每个任务完成后必须 `cargo build -p dozer-app`、相关 `cargo test -p dozer-app <mod>` 全过、`cargo clippy -p dozer-app --all-targets -- -D warnings` 干净、`cargo fmt -p dozer-app -- --check` 干净,才能进入下一个任务。
- 沿用仓库既有的中文注释风格与"纯函数拆出来方便 headless 单测"的惯例(`delivery.rs`/`git_log.rs` 已是这个风格)。

---

## 参考:改动前的相关代码位置

- `crates/dozer-app/src/delivery.rs`:`FileStatus`(11-129 行区间)、`file_statuses`(134-159)、`dir_status`(163-174)、`branch`(177-181)、`is_dirty`(40-42)。
- `crates/dozer-app/src/workspace.rs`:`use crate::delivery::{self, FileChange, FileStatus};`(34 行)、`Message::ProjectGitRefreshed`(1063-1068)、`Workspace.git_statuses` 字段(1384)、`Workspace::spawn_project_git_refresh`(1813-1832)、`Message::ProjectGitRefreshed` 处理分支(3785-3791)、文件树行渲染里的 `dir_status`/`git_statuses.get` 调用(6250-6254)与色点渲染(6306-6312)、`tree_row_dot`(7383-7389)、对应测试(8672-8674)。
- `crates/dozer-app/src/project.rs`:`HIDDEN` 名单(15 行:`[".git", "target", "node_modules", ".DS_Store"]`)。
- `crates/dozer-app/Cargo.toml`:已有 `gleisbau = "0.7"`(spike 引入,带出 `git2 = "0.21"` 传递依赖)。

---

### Task 1: `delivery.rs` 加 `git2` 直接依赖,`file_statuses` 迁移到 git2 并升级为暂存/工作区双态

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`
- Modify: `crates/dozer-app/src/delivery.rs:11-159`(`FileStatus` 定义与 `file_statuses`)
- Modify: `crates/dozer-app/src/delivery.rs:286-316`(既有测试 `file_statuses_maps_modified_new_deleted`)

**Interfaces:**
- Produces: `pub enum ChangeKind { New, Modified, Deleted }`、`pub struct FileGitStatus { pub kind: ChangeKind, pub staged: bool, pub unstaged: bool }`、`pub fn file_statuses(repo: &Path) -> HashMap<PathBuf, FileGitStatus>`(与旧版同名,返回值类型变了——下游 Task 6/7 会跟着改)。

- [ ] **Step 1: 加 `git2` 为直接依赖**

编辑 `crates/dozer-app/Cargo.toml`,在 `gleisbau = "0.7"` 那一行下面加:

```toml
git2 = "0.21"
```

（版本号必须和 `gleisbau` 传递依赖的一致,否则 cargo 会同时拉两份 `git2`。用 `cargo tree -p dozer-app -i git2` 确认迁移后只有一份。）

- [ ] **Step 2: 写新的失败测试(暂存/工作区双态)**

在 `crates/dozer-app/src/delivery.rs` 的 `#[cfg(test)] mod tests` 里,紧跟在既有 `file_statuses_maps_modified_new_deleted` 测试后面加:

```rust
#[test]
fn file_statuses_distinguishes_staged_and_unstaged() {
    let (_d, repo) = mkrepo(); // a.txt 已提交一次
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
    };

    // 只暂存,不叠加工作区改动
    std::fs::write(repo.join("a.txt"), "staged-only\n").unwrap();
    git(&["add", "a.txt"]);
    let m = file_statuses(&repo);
    let a = m.get(&repo.join("a.txt")).expect("a.txt 应有状态");
    assert!(a.staged && !a.unstaged, "纯暂存: {a:?}");
    assert_eq!(a.kind, ChangeKind::Modified);

    // 再叠加一次未暂存改动(对应旧 porcelain 的 MM)
    std::fs::write(repo.join("a.txt"), "staged-only\nplus more\n").unwrap();
    let m2 = file_statuses(&repo);
    let a2 = m2.get(&repo.join("a.txt")).expect("a.txt 应有状态");
    assert!(a2.staged && a2.unstaged, "部分暂存+未暂存: {a2:?}");

    // 纯工作区改动:新建但没 git add 的文件
    std::fs::write(repo.join("new.txt"), "n\n").unwrap();
    let m3 = file_statuses(&repo);
    let n = m3.get(&repo.join("new.txt")).expect("new.txt 应有状态");
    assert!(!n.staged && n.unstaged, "未跟踪: {n:?}");
    assert_eq!(n.kind, ChangeKind::New);
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p dozer-app delivery::tests::file_statuses_distinguishes_staged_and_unstaged`
Expected: 编译失败(`FileGitStatus`/`ChangeKind` 不存在)——这是预期中的失败,证明测试确实在检验还没写的东西。

- [ ] **Step 4: 用 git2 重写 `FileStatus`/`file_statuses`**

把 `crates/dozer-app/src/delivery.rs` 里下面这段(11-17 行的 `FileChange` 之后、124-159 行的旧 `FileStatus`/`file_statuses`):

```rust
/// 文件树装饰用的 git 状态（P1h）。粗粒度三态,不分暂存/工作区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    New,
    Deleted,
}

/// `git status --porcelain` → 绝对路径 → 状态。非 git / 失败返回空。
/// 码映射:`??`/含 `A`→New;含 `D`→Deleted;其余→Modified。重命名取箭头后的新名。
pub fn file_statuses(repo: &Path) -> HashMap<PathBuf, FileStatus> {
    let mut map = HashMap::new();
    let Some(out) = git(repo, &["status", "--porcelain"]) else {
        return map;
    };
    for line in out.lines() {
        if line.len() < 4 {
            continue;
        }
        let code = &line[..2];
        let rest = &line[3..];
        let path = rest.rsplit(" -> ").next().unwrap_or(rest).trim();
        if path.is_empty() {
            continue;
        }
        let status = if code == "??" || code.contains('A') {
            FileStatus::New
        } else if code.contains('D') {
            FileStatus::Deleted
        } else {
            FileStatus::Modified
        };
        map.insert(repo.join(path), status);
    }
    map
}
```

整段替换成:

```rust
/// 文件树装饰用的 git 改动类型(P1h 三态,D2 起不再单独承担暂存/工作区
/// 区分——那部分挪进 [`FileGitStatus::staged`]/[`unstaged`]())。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    New,
    Modified,
    Deleted,
}

/// 单个文件的 git 状态:`kind` 决定色点颜色,`staged`/`unstaged` 决定色点
/// 填充态(见 workspace.rs `tree_row_dot_glyph`)。二者可同时为真(对应旧
/// porcelain 码里的 `MM`:部分暂存 + 又有新改动)。只要这个文件出现在
/// `file_statuses` 返回的 map 里,`staged`/`unstaged` 至少一个为真。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileGitStatus {
    pub kind: ChangeKind,
    pub staged: bool,
    pub unstaged: bool,
}

/// 用 `git2::Repository::statuses` 取代 shell 出 `git status --porcelain`
/// 文本解析(D1):`INDEX_*` 标志位=相对 HEAD 的暂存改动,`WT_*`=相对 index
/// 的工作区改动,原生区分,不用再猜双字符码语义。非 git / 打开失败返回空。
pub fn file_statuses(repo: &Path) -> HashMap<PathBuf, FileGitStatus> {
    let mut map = HashMap::new();
    let Ok(git_repo) = git2::Repository::open(repo) else {
        return map;
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let Ok(statuses) = git_repo.statuses(Some(&mut opts)) else {
        return map;
    };
    for entry in statuses.iter() {
        let Ok(rel) = entry.path() else {
            continue; // 非 UTF-8 路径,跳过而非崩(同旧版"格式意外跳过该行"精神)
        };
        if rel.is_empty() {
            continue;
        }
        let s = entry.status();
        if s.contains(git2::Status::IGNORED) {
            continue;
        }
        let staged = s.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        );
        let unstaged = s.intersects(
            git2::Status::WT_NEW
                | git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED
                | git2::Status::WT_TYPECHANGE,
        );
        if !staged && !unstaged {
            continue;
        }
        let kind = if s.intersects(git2::Status::INDEX_NEW | git2::Status::WT_NEW) {
            ChangeKind::New
        } else if s.intersects(git2::Status::INDEX_DELETED | git2::Status::WT_DELETED) {
            ChangeKind::Deleted
        } else {
            ChangeKind::Modified
        };
        map.insert(
            repo.join(rel),
            FileGitStatus {
                kind,
                staged,
                unstaged,
            },
        );
    }
    map
}
```

- [ ] **Step 5: 更新既有测试引用旧类型名的地方**

`crates/dozer-app/src/delivery.rs` 里的 `file_statuses_maps_modified_new_deleted` 测试(286-316 行附近)当前是:

```rust
let m = file_statuses(&repo);
assert_eq!(m.get(&repo.join("a.txt")), Some(&FileStatus::Modified));
assert_eq!(m.get(&repo.join("new.txt")), Some(&FileStatus::New));
assert_eq!(m.get(&repo.join("c.txt")), Some(&FileStatus::Deleted));
assert!(
    file_statuses(std::path::Path::new("/")).is_empty(),
    "非 git 空"
);
```

改成:

```rust
let m = file_statuses(&repo);
assert_eq!(m.get(&repo.join("a.txt")).map(|s| s.kind), Some(ChangeKind::Modified));
assert_eq!(m.get(&repo.join("new.txt")).map(|s| s.kind), Some(ChangeKind::New));
assert_eq!(m.get(&repo.join("c.txt")).map(|s| s.kind), Some(ChangeKind::Deleted));
assert!(
    file_statuses(std::path::Path::new("/")).is_empty(),
    "非 git 空"
);
```

- [ ] **Step 6: 跑测试确认全过**

Run: `cargo test -p dozer-app delivery::`
Expected: `file_statuses_maps_modified_new_deleted`、`file_statuses_distinguishes_staged_and_unstaged` 均 PASS。此时 `dir_status`/`branch` 相关测试会因为类型不匹配编译失败——这是预期的,Task 2/3 会修。先确认这两个新/改测试本身逻辑对。如果整个 crate 因为下游编译错误跑不起来,改用 `cargo test -p dozer-app --lib delivery::tests::file_statuses_distinguishes_staged_and_unstaged -- --exact` 单独跑通过再继续。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/Cargo.toml crates/dozer-app/src/delivery.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): file_statuses 迁移到 git2,区分暂存/工作区改动

FileStatus 三态拆成 ChangeKind(色) + staged/unstaged(填充态),数据源
从 shell 出 git status --porcelain 换成 git2::Repository::statuses,
原生拿 INDEX_*/WT_* 标志位,不用再猜双字符码语义。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

（此时 `cargo build -p dozer-app` 预期仍会因为 `dir_status`/`branch`/`workspace.rs` 里的旧类型引用而失败,这是正常的中间态——Task 2/3/6/7 会依次修完。不要在这一步尝试让整个 crate 编译通过。）

---

### Task 2: `branch`/`is_dirty` 迁移到 git2

**Files:**
- Modify: `crates/dozer-app/src/delivery.rs:40-42`(`is_dirty`)、`176-181`(`branch`)

**Interfaces:**
- Consumes: 无(不依赖 Task 1 的类型,只是同一个文件里紧邻的另外两个函数)。
- Produces: `pub fn branch(repo: &Path) -> Option<String>`、`pub fn is_dirty(repo: &Path) -> bool`——签名与旧版完全一致,调用方(`workspace.rs` 多处)不用改。

- [ ] **Step 1: 写失败测试(HEAD 分离态)**

在 `mod tests` 里紧跟 `branch_of_repo` 测试后加一个新场景——git2 版本要能正确处理"detached HEAD"(旧版靠 `line != "HEAD"` 判断,git2 有专门 API,行为要保持一致):

```rust
#[test]
fn branch_none_when_head_detached() {
    let (_d, repo) = mkrepo();
    let head = head_commit(&repo).unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap()
    };
    git(&["checkout", "-q", &head]); // detach HEAD 到当前 commit
    assert!(branch(&repo).is_none(), "detached HEAD 不该报出分支名");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app delivery::tests::branch_none_when_head_detached`
Expected: 用旧版 CLI 实现应该已经能过(`rev-parse --abbrev-ref HEAD` 在 detached 态输出字面量 `HEAD`,旧代码 `line != "HEAD"` 那行专门处理了这个)。这一步是回归测试,不是新失败——先确认现状真的过,再进 Step 3 改实现后必须仍然过。

- [ ] **Step 3: 用 git2 重写 `is_dirty`/`branch`**

把:

```rust
pub fn is_dirty(repo: &Path) -> bool {
    git(repo, &["status", "--porcelain"]).is_some_and(|s| !s.trim().is_empty())
}
```

改成:

```rust
/// 工作区是否有任何改动(暂存或未暂存,不含被 `.gitignore` 排除的文件)。
pub fn is_dirty(repo: &Path) -> bool {
    let Ok(git_repo) = git2::Repository::open(repo) else {
        return false;
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    git_repo
        .statuses(Some(&mut opts))
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}
```

把:

```rust
/// 当前分支名（`git rev-parse --abbrev-ref HEAD`）；非 git / 无提交返回 None。
pub fn branch(repo: &Path) -> Option<String> {
    let out = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let line = out.lines().next()?.trim();
    (!line.is_empty() && line != "HEAD").then(|| line.to_string())
}
```

改成:

```rust
/// 当前分支名;非 git / 无提交 / detached HEAD 返回 None。
pub fn branch(repo: &Path) -> Option<String> {
    let git_repo = git2::Repository::open(repo).ok()?;
    let head = git_repo.head().ok()?;
    if !head.is_branch() {
        return None; // detached HEAD 或指向 tag 等非分支引用
    }
    head.shorthand().map(str::to_string)
}
```

- [ ] **Step 4: 跑全部 delivery 测试确认通过**

Run: `cargo test -p dozer-app delivery::`
Expected: `repo_root_and_head_and_dirty`、`branch_of_repo`、`branch_none_when_head_detached`、Task 1 新增的两个测试均 PASS。`dir_status_aggregates_new_vs_modified` 此时仍会编译失败(用的是旧 `FileStatus`),留给 Task 3。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/delivery.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): branch/is_dirty 迁移到 git2

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `dir_status` 升级为 `DirGitStatus` rollup

**Files:**
- Modify: `crates/dozer-app/src/delivery.rs:161-174`(`dir_status`)、`318-332`(既有测试 `dir_status_aggregates_new_vs_modified`)

**Interfaces:**
- Consumes: `ChangeKind`、`FileGitStatus`(Task 1)。
- Produces: `pub struct DirGitStatus { pub kind: ChangeKind, pub staged: bool, pub unstaged: bool }`、`pub fn dir_status(dir: &Path, statuses: &HashMap<PathBuf, FileGitStatus>) -> Option<DirGitStatus>`。

- [ ] **Step 1: 写失败测试(混合暂存态的 rollup)**

在 `mod tests` 里紧跟(改名后的)`dir_status_aggregates_new_vs_modified` 测试后加:

```rust
#[test]
fn dir_status_rolls_up_staged_and_unstaged() {
    use std::path::{Path, PathBuf};
    let mut s = HashMap::new();
    s.insert(
        PathBuf::from("/r/src/a.rs"),
        FileGitStatus { kind: ChangeKind::Modified, staged: true, unstaged: false },
    );
    s.insert(
        PathBuf::from("/r/src/b.rs"),
        FileGitStatus { kind: ChangeKind::New, staged: false, unstaged: true },
    );
    let rollup = dir_status(Path::new("/r/src"), &s).expect("有改动的目录应有 rollup");
    assert_eq!(rollup.kind, ChangeKind::Modified, "Modified 优先级高于 New");
    assert!(rollup.staged, "子文件里有一个暂存了");
    assert!(rollup.unstaged, "子文件里有一个只在工作区");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app delivery::tests::dir_status_rolls_up_staged_and_unstaged`
Expected: FAIL(`DirGitStatus`/新签名不存在)。

- [ ] **Step 3: 重写 `dir_status`**

把:

```rust
/// 目录（含深层）的聚合 git 状态（rollup）：底下有"改/删"→Modified(金)、
/// 只有"新"→New(绿)、无变更→None。让新建目录显绿而非误标金。
pub fn dir_status(dir: &Path, statuses: &HashMap<PathBuf, FileStatus>) -> Option<FileStatus> {
    let mut found_new = false;
    for (path, st) in statuses {
        if path.starts_with(dir) {
            match st {
                FileStatus::Modified | FileStatus::Deleted => return Some(FileStatus::Modified),
                FileStatus::New => found_new = true,
            }
        }
    }
    found_new.then_some(FileStatus::New)
}
```

改成:

```rust
/// 目录(含深层)的聚合 git 状态(rollup):`kind` 取子孙中最"重"的
/// (Modified/Deleted > New,与 P1h 现状一致,让新建目录显绿而非误标金);
/// `staged`/`unstaged` 只要有任一子孙为真就为真(D2)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirGitStatus {
    pub kind: ChangeKind,
    pub staged: bool,
    pub unstaged: bool,
}

pub fn dir_status(dir: &Path, statuses: &HashMap<PathBuf, FileGitStatus>) -> Option<DirGitStatus> {
    let mut found_new = false;
    let mut heavy: Option<ChangeKind> = None;
    let mut staged = false;
    let mut unstaged = false;
    for (path, st) in statuses {
        if !path.starts_with(dir) {
            continue;
        }
        staged |= st.staged;
        unstaged |= st.unstaged;
        match st.kind {
            ChangeKind::Modified | ChangeKind::Deleted => heavy = Some(ChangeKind::Modified),
            ChangeKind::New => found_new = true,
        }
    }
    let kind = heavy.or(found_new.then_some(ChangeKind::New))?;
    Some(DirGitStatus { kind, staged, unstaged })
}
```

- [ ] **Step 4: 更新既有测试到新类型**

把 `dir_status_aggregates_new_vs_modified` 测试(318-332 行附近)整段改成:

```rust
#[test]
fn dir_status_aggregates_new_vs_modified() {
    use std::path::{Path, PathBuf};
    let mut s = HashMap::new();
    s.insert(
        PathBuf::from("/r/logo/a.png"),
        FileGitStatus { kind: ChangeKind::New, staged: false, unstaged: true },
    );
    s.insert(
        PathBuf::from("/r/logo/b.png"),
        FileGitStatus { kind: ChangeKind::New, staged: false, unstaged: true },
    );
    s.insert(
        PathBuf::from("/r/src/main.rs"),
        FileGitStatus { kind: ChangeKind::Modified, staged: false, unstaged: true },
    );
    assert_eq!(dir_status(Path::new("/r/logo"), &s).map(|d| d.kind), Some(ChangeKind::New));
    assert_eq!(dir_status(Path::new("/r/src"), &s).map(|d| d.kind), Some(ChangeKind::Modified));
    assert_eq!(dir_status(Path::new("/r"), &s).map(|d| d.kind), Some(ChangeKind::Modified));
    assert!(dir_status(Path::new("/r/docs"), &s).is_none());
}
```

- [ ] **Step 5: 跑全部 delivery 测试确认通过**

Run: `cargo test -p dozer-app delivery::`
Expected: 全过。此时 `cargo build -p dozer-app` 会在 `workspace.rs` 报一堆类型不匹配错误(用的还是旧 `FileStatus`)——这是预期的,Task 6/7 会修,先不管。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/delivery.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): dir_status rollup 升级为暂存/工作区双态

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: `WorktreeInfo` + `worktrees()`

**Files:**
- Modify: `crates/dozer-app/src/delivery.rs`(文件末尾,`branch` 函数之后)

**Interfaces:**
- Consumes: 无。
- Produces:
  ```rust
  pub struct WorktreeInfo {
      pub name: String,
      pub path: PathBuf,
      pub branch: Option<String>,
      pub dirty: bool,
      pub is_current: bool,
      pub missing: bool,
  }
  pub fn worktrees(repo: &Path) -> Vec<WorktreeInfo>
  ```

- [ ] **Step 1: 写失败测试(主 worktree + 2 个链接 worktree)**

在 `mod tests` 里加(需要 `git worktree add`,复用 `mkrepo` 的 `git` helper 模式):

```rust
#[test]
fn worktrees_lists_main_and_linked() {
    let (_d, repo) = mkrepo(); // main 分支,a.txt 已提交
    let git = |args: &[&str]| {
        let st = std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(st.status.success(), "git {args:?}: {st:?}");
    };
    let parent = repo.parent().unwrap();
    let wt1 = parent.join("wt1");
    let wt2 = parent.join("wt2");
    git(&["branch", "feature-a"]);
    git(&[
        "worktree",
        "add",
        wt1.to_str().unwrap(),
        "feature-a",
    ]);
    git(&["branch", "feature-b"]);
    git(&[
        "worktree",
        "add",
        wt2.to_str().unwrap(),
        "feature-b",
    ]);
    std::fs::write(wt1.join("a.txt"), "dirty in wt1\n").unwrap(); // wt1 弄脏

    let list = worktrees(&repo);
    assert_eq!(list.len(), 3, "主 worktree + 2 个链接: {list:?}");

    let main = list.iter().find(|w| w.is_current).expect("应有当前项");
    assert_eq!(main.path.canonicalize().unwrap(), repo.canonicalize().unwrap());
    assert_eq!(main.branch.as_deref(), Some("main"));

    let w1 = list
        .iter()
        .find(|w| w.path.canonicalize().unwrap() == wt1.canonicalize().unwrap())
        .expect("应找到 wt1");
    assert_eq!(w1.branch.as_deref(), Some("feature-a"));
    assert!(w1.dirty);
    assert!(!w1.is_current);
    assert!(!w1.missing);

    let w2 = list
        .iter()
        .find(|w| w.path.canonicalize().unwrap() == wt2.canonicalize().unwrap())
        .expect("应找到 wt2");
    assert_eq!(w2.branch.as_deref(), Some("feature-b"));
    assert!(!w2.dirty);
}

#[test]
fn worktrees_marks_missing_when_directory_deleted() {
    let (_d, repo) = mkrepo();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap()
    };
    let parent = repo.parent().unwrap();
    let wt = parent.join("wt-missing");
    git(&["branch", "gone"]);
    git(&["worktree", "add", wt.to_str().unwrap(), "gone"]);
    std::fs::remove_dir_all(&wt).unwrap(); // 手动删掉目录,不走 `git worktree remove`

    let list = worktrees(&repo);
    let missing = list
        .iter()
        .find(|w| w.name == "wt-missing")
        .expect("git 元数据还在,不该从列表消失");
    assert!(missing.missing);
    assert!(missing.branch.is_none());
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app delivery::tests::worktrees_lists_main_and_linked`
Expected: FAIL(`worktrees` 不存在)。

- [ ] **Step 3: 实现 `worktrees()`**

在 `delivery.rs` 文件末尾(`branch` 函数之后、`delivery_pending` 之前)加:

```rust
/// 同仓库的其他 git worktree(D3)。主 worktree 靠 `commondir().parent()`
/// 推导(对主/链接 worktree 都成立:`commondir` 恒指向主仓库的 `.git`),
/// 链接 worktree 靠 `Repository::worktrees()` 列出(libgit2 只列链接的,
/// 不含主 worktree,所以主项要单独插入)。
pub fn worktrees(repo: &Path) -> Vec<WorktreeInfo> {
    let Ok(git_repo) = git2::Repository::open(repo) else {
        return Vec::new();
    };
    let mut out = Vec::new();

    let Some(main_workdir) = git_repo.commondir().parent().map(Path::to_path_buf) else {
        return Vec::new();
    };
    let main_name = main_workdir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let is_current_main = repo
        .canonicalize()
        .ok()
        .zip(main_workdir.canonicalize().ok())
        .is_some_and(|(a, b)| a == b);
    out.push(WorktreeInfo {
        name: main_name,
        branch: branch(&main_workdir),
        dirty: is_dirty(&main_workdir),
        is_current: is_current_main,
        missing: false,
        path: main_workdir,
    });

    let Ok(names) = git_repo.worktrees() else {
        return out;
    };
    for name in names.iter().flatten() {
        let Ok(wt) = git_repo.find_worktree(name) else {
            continue;
        };
        let path = wt.path().to_path_buf();
        if !path.is_dir() {
            out.push(WorktreeInfo {
                name: name.to_string(),
                path,
                branch: None,
                dirty: false,
                is_current: false,
                missing: true,
            });
            continue;
        }
        let is_current = repo
            .canonicalize()
            .ok()
            .zip(path.canonicalize().ok())
            .is_some_and(|(a, b)| a == b);
        out.push(WorktreeInfo {
            name: name.to_string(),
            branch: branch(&path),
            dirty: is_dirty(&path),
            is_current,
            missing: false,
            path,
        });
    }
    out
}

/// 同仓库的一个 worktree(主或链接)。见 [`worktrees`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub name: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub dirty: bool,
    pub is_current: bool,
    /// worktree 元数据还在(`.git/worktrees/<name>` 存在)但工作目录已被
    /// 手动删除/移走(没走 `git worktree remove`)。
    pub missing: bool,
}
```

（注意 `WorktreeInfo` 结构体定义写在 `worktrees` 函数后面——Rust 里顺序不影响编译,但如果你的编辑器/linter 有"类型先声明后使用"的风格偏好,把结构体挪到函数前面也可以,不影响行为。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app delivery::tests::worktrees_`
Expected: `worktrees_lists_main_and_linked`、`worktrees_marks_missing_when_directory_deleted` 均 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/delivery.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): delivery::worktrees 列出同仓库的其他 git worktree

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: `git_watch.rs` —— debounced `notify` 文件系统监听

**Files:**
- Create: `crates/dozer-app/src/git_watch.rs`
- Modify: `crates/dozer-app/Cargo.toml`(加 `notify` 依赖)
- Modify: `crates/dozer-app/src/main.rs`(加 `mod git_watch;`)

**Interfaces:**
- Consumes: `crate::project::HIDDEN`(`[".git", "target", "node_modules", ".DS_Store"]`)。
- Produces:
  ```rust
  pub fn is_relevant_path(repo: &Path, changed: &Path) -> Option<Relevance>
  pub enum Relevance { Workdir, GitRefs }
  pub struct Handle { /* Drop 时自动 unwatch,字段不对外公开 */ }
  pub fn start(
      handle: &tokio::runtime::Handle,
      repo: PathBuf,
      debounce: std::time::Duration,
      mut on_change: impl FnMut(Relevance) + Send + 'static,
  ) -> notify::Result<Handle>
  ```
  Task 6 会用 `Relevance::GitRefs` 决定要不要顺带重建 Git Log 快照(Plan 2 里接线),Task 6 本身只需要"发生了变化就重跑 `spawn_project_git_refresh`",两种 `Relevance` 都触发。

- [ ] **Step 1: 加 `notify` 依赖**

编辑 `crates/dozer-app/Cargo.toml`,在 Task 1 加的 `git2 = "0.21"` 下面加:

```toml
notify = "8"
```

- [ ] **Step 2: 写 `is_relevant_path` 的失败测试(纯函数,不碰真实文件系统)**

创建 `crates/dozer-app/src/git_watch.rs`,先写测试骨架(会编译失败,因为函数体还没写):

```rust
// crates/dozer-app/src/git_watch.rs
//! 项目工作区的实时文件系统监听(D4):debounce 后触发一次 git 状态刷新,
//! 不用再等用户手动切页签/等回合结束。

use std::path::{Path, PathBuf};
use std::time::Duration;

/// 一次文件系统事件相对这个仓库"值不值得触发刷新"、触发的话算哪一类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relevance {
    /// 工作区文件改动(status/暂存态可能变了)。
    Workdir,
    /// `.git` 引用类文件变了(HEAD/index/refs/packed-refs)——分支切换、
    /// 外部提交、其他 worktree 提交都会碰这几个文件,除了刷新文件树状态,
    /// 调用方通常还想顺带重建 Git Log 快照。
    GitRefs,
}

/// `changed` 是否值得触发刷新,值得的话是哪一类。`.git` 目录整体在
/// `project::HIDDEN` 排除名单里,但其中的 HEAD/index/refs/packed-refs
/// 要单独放行(D4)。
fn is_relevant_path(repo: &Path, changed: &Path) -> Option<Relevance> {
    let rel = changed.strip_prefix(repo).ok()?;
    let mut parts = rel.components();
    let Some(std::path::Component::Normal(first)) = parts.next() else {
        return Some(Relevance::Workdir); // 仓库根自身的事件,极少见,当作相关
    };
    let first = first.to_string_lossy();
    if first == ".git" {
        let rest: PathBuf = parts.collect();
        let rest_str = rest.to_string_lossy();
        if rest_str == "HEAD" || rest_str == "index" || rest_str == "packed-refs" {
            return Some(Relevance::GitRefs);
        }
        if rest.starts_with("refs") {
            return Some(Relevance::GitRefs);
        }
        return None; // .git 下其余内容(objects/ 等)不关心
    }
    if crate::project::HIDDEN.contains(&first.as_ref()) {
        return None;
    }
    Some(Relevance::Workdir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workdir_file_is_relevant() {
        let repo = Path::new("/r");
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/src/main.rs")),
            Some(Relevance::Workdir)
        );
    }

    #[test]
    fn hidden_dirs_are_not_relevant() {
        let repo = Path::new("/r");
        assert_eq!(is_relevant_path(repo, Path::new("/r/target/debug/foo")), None);
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/node_modules/x/index.js")),
            None
        );
        assert_eq!(is_relevant_path(repo, Path::new("/r/.DS_Store")), None);
    }

    #[test]
    fn git_control_files_are_relevant_as_git_refs() {
        let repo = Path::new("/r");
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/.git/HEAD")),
            Some(Relevance::GitRefs)
        );
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/.git/index")),
            Some(Relevance::GitRefs)
        );
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/.git/refs/heads/main")),
            Some(Relevance::GitRefs)
        );
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/.git/packed-refs")),
            Some(Relevance::GitRefs)
        );
    }

    #[test]
    fn other_git_internals_are_not_relevant() {
        let repo = Path::new("/r");
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/.git/objects/ab/cdef")),
            None
        );
    }
}
```

- [ ] **Step 3: 注册模块,跑测试**

编辑 `crates/dozer-app/src/main.rs`,在 `mod fonts;` 和 `mod git_log;` 之间加(按现有字母序排列的惯例):

```rust
mod git_watch;
```

Run: `cargo test -p dozer-app git_watch::`
Expected: 4 个测试全 PASS(`is_relevant_path` 是纯函数,不需要真实文件系统)。

- [ ] **Step 4: Commit(纯函数部分先落地)**

```bash
git add crates/dozer-app/Cargo.toml crates/dozer-app/src/main.rs crates/dozer-app/src/git_watch.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): git_watch::is_relevant_path 判定文件变化是否触发刷新

纯函数打底,真实 notify 监听器接下来一个提交加。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 5: 写真实 notify 监听 + debounce 的失败测试**

在 `git_watch.rs` 的 `mod tests` 里加(用真实 tempdir + 真实文件写入,断言 debounce 合并多次写入成一次回调):

```rust
#[test]
fn start_debounces_rapid_writes_into_one_callback() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_path_buf();
    std::fs::create_dir_all(repo.join(".git")).unwrap(); // 让 .git 存在,贴近真实仓库

    let rt = tokio::runtime::Runtime::new().unwrap();
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count2 = count.clone();

    let _handle = rt.block_on(async {
        start(
            &tokio::runtime::Handle::current(),
            repo.clone(),
            Duration::from_millis(100),
            move |_relevance| {
                count2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
        )
        .expect("watcher 应能启动")
    });

    // 100ms debounce 窗口内连续写 5 次
    for i in 0..5 {
        std::fs::write(repo.join(format!("f{i}.txt")), "x").unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
    // 等 debounce 窗口过完 + 一点余量
    rt.block_on(async { tokio::time::sleep(Duration::from_millis(300)).await });

    assert_eq!(
        count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "5 次快速写入应合并成 1 次回调"
    );
}

#[test]
fn start_ignores_hidden_dir_writes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_path_buf();
    std::fs::create_dir_all(repo.join("target")).unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count2 = count.clone();

    let _handle = rt.block_on(async {
        start(
            &tokio::runtime::Handle::current(),
            repo.clone(),
            Duration::from_millis(100),
            move |_relevance| {
                count2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
        )
        .unwrap()
    });

    std::fs::write(repo.join("target").join("build-artifact"), "x").unwrap();
    rt.block_on(async { tokio::time::sleep(Duration::from_millis(300)).await });

    assert_eq!(
        count.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "target/ 下的改动不该触发回调"
    );
}
```

- [ ] **Step 6: 跑测试确认失败**

Run: `cargo test -p dozer-app git_watch::tests::start_debounces_rapid_writes_into_one_callback`
Expected: FAIL(`start`/`Handle` 不存在)。

- [ ] **Step 7: 实现 `start`/`Handle`**

在 `git_watch.rs` 里,`is_relevant_path` 函数后面(`#[cfg(test)]` 之前)加:

```rust
/// 一个存活的监听器。Drop 时自动停止(RAII)——调用方(`Workspace`)不需要
/// 在项目切换/关闭的每个路径上手动喊停,`Workspace` 自己被 drop 掉的时候
/// 这个字段跟着 drop,监听自然停。
pub struct Handle {
    _watcher: notify::RecommendedWatcher,
}

/// 启动一个仓库根的 debounced 监听。`on_change` 在 debounce 窗口(连续事件
/// 间隔小于 `debounce` 就算同一批)结束后,拿这批事件里"最值得上报"的
/// `Relevance` 调用一次——`GitRefs` 优先于 `Workdir`(一批事件里只要有一个
/// 引用类变化,调用方就该顺带重建 Git Log 快照)。
///
/// 内部用 `tokio::sync::mpsc` 把 notify 的同步回调(跑在 notify 自己的后台
/// 线程上)和这里的 debounce 循环(跑在调用方传入的 tokio runtime 上)串起来,
/// 不引入 `notify-debouncer-*` 系列额外依赖(规格 §4 只 accept 了 `notify`
/// 本身)。
pub fn start(
    handle: &tokio::runtime::Handle,
    repo: PathBuf,
    debounce: Duration,
    mut on_change: impl FnMut(Relevance) + Send + 'static,
) -> notify::Result<Handle> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Relevance>();
    let filter_repo = repo.clone();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        let mut best: Option<Relevance> = None;
        for path in &event.paths {
            match is_relevant_path(&filter_repo, path) {
                Some(Relevance::GitRefs) => {
                    best = Some(Relevance::GitRefs);
                    break; // GitRefs 优先级最高,找到就不用再看这批里其余路径
                }
                Some(Relevance::Workdir) => best = best.or(Some(Relevance::Workdir)),
                None => {}
            }
        }
        if let Some(r) = best {
            let _ = tx.send(r);
        }
    })?;
    watcher.watch(&repo, notify::RecursiveMode::Recursive)?;

    handle.spawn(async move {
        while let Some(first) = rx.recv().await {
            let mut best = first;
            // 合并这个 debounce 窗口内接下来到达的信号
            loop {
                match tokio::time::timeout(debounce, rx.recv()).await {
                    Ok(Some(r)) => {
                        if r == Relevance::GitRefs {
                            best = Relevance::GitRefs;
                        }
                    }
                    _ => break, // 超时(这一批安静下来了)或 channel 关闭
                }
            }
            on_change(best);
        }
    });

    Ok(Handle { _watcher: watcher })
}
```

- [ ] **Step 8: 跑测试确认通过**

Run: `cargo test -p dozer-app git_watch::`
Expected: 6 个测试全 PASS(4 个纯函数 + 2 个真实 notify 集成测试)。真实文件系统测试在 CI/慢机器上可能偶发超时,如果 `start_debounces_rapid_writes_into_one_callback` 偶尔因为等待时间不够失败,把两处 `Duration::from_millis(300)` 加到 `500` 再试一次——不要把 debounce 窗口本身(100ms)改大,那会拖慢真实使用时的响应感。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/git_watch.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): git_watch::start 起一个 debounced notify 监听器

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: `workspace.rs` 接线——`Message`/`Workspace` 字段升级,watcher 生命周期

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:34`(import)
- Modify: `crates/dozer-app/src/workspace.rs:1063-1068`(`Message::ProjectGitRefreshed`)
- Modify: `crates/dozer-app/src/workspace.rs:1384`(`Workspace.git_statuses` 字段区)
- Modify: `crates/dozer-app/src/workspace.rs:1538-1624`(`Workspace::bootstrap`/`from_restore`)
- Modify: `crates/dozer-app/src/workspace.rs:1635-1668`(`empty_for_project_placeholder`)
- Modify: `crates/dozer-app/src/workspace.rs:1813-1832`(`spawn_project_git_refresh`)
- Modify: `crates/dozer-app/src/workspace.rs:3785-3791`(`Message::ProjectGitRefreshed` 处理分支)
- Modify: `crates/dozer-app/src/workspace.rs`(`Message` 枚举里加一个新 variant;处理分支加一段)

**Interfaces:**
- Consumes: `delivery::{FileGitStatus, WorktreeInfo, worktrees}`(Task 1/4)、`git_watch::{start, Handle, Relevance}`(Task 5)。
- Produces:`Workspace.worktrees: Vec<delivery::WorktreeInfo>`(公开只读,Plan 2 的 Git Log 面板要读)、`Message::ProjectFsChanged(ProjectId, git_watch::Relevance)`(Plan 2 会在处理这个消息时额外接一段"若 `Relevance::GitRefs` 则重建 Git Log 快照",本任务只负责触发既有的 `spawn_project_git_refresh`)。

- [ ] **Step 1: 升级 import 与 `Message::ProjectGitRefreshed`**

第 34 行:

```rust
use crate::delivery::{self, FileChange, FileStatus};
```

改成:

```rust
use crate::delivery::{self, FileChange, FileGitStatus, WorktreeInfo};
use crate::git_watch;
```

第 1063-1068 行:

```rust
    /// 项目:git 分支/脏/文件状态刷新结果。
    ProjectGitRefreshed(
        ProjectId,
        Option<String>,
        bool,
        HashMap<PathBuf, FileStatus>,
    ),
```

改成:

```rust
    /// 项目:git 分支/脏/文件状态/worktree 列表刷新结果。
    ProjectGitRefreshed(
        ProjectId,
        Option<String>,
        bool,
        HashMap<PathBuf, FileGitStatus>,
        Vec<WorktreeInfo>,
    ),
    /// 项目:`git_watch` 监听到工作区/`.git` 引用变化,该重新跑一次 git 刷新
    /// 了(D4)。`Relevance` 决定这次触发要不要顺带做 Plan 2 的 Git Log 快照
    /// 重建。
    ProjectFsChanged(ProjectId, git_watch::Relevance),
```

在紧邻这两行的地方找 `Workspace.git_statuses` 字段声明(约 1384 行):

```rust
    git_statuses: HashMap<PathBuf, FileStatus>,
```

改成:

```rust
    git_statuses: HashMap<PathBuf, FileGitStatus>,
    /// 同仓库的其他 git worktree(D3),随 git 刷新一起更新。
    worktrees: Vec<WorktreeInfo>,
    /// 本项目的实时文件系统监听(D4)。`None` 只可能出现在 watcher 启动
    /// 失败时(降级为"只在开项目/回合结束时刷新")。Drop 时自动停止。
    git_watch: Option<git_watch::Handle>,
```

- [ ] **Step 2: `empty_for_project_placeholder` 补新字段**

在 1635-1668 行的 `empty_for_project_placeholder` 函数体里,紧跟 `git_statuses: HashMap::new(),` 那一行加:

```rust
            git_statuses: HashMap::new(),
            worktrees: Vec::new(),
            git_watch: None,
```

- [ ] **Step 3: `spawn_project_git_refresh` 一并取 worktrees**

把 1813-1832 行:

```rust
    /// 异步刷新当前项目的 git 分支/脏/文件状态（打开项目 + 回合结束触发）。
    fn spawn_project_git_refresh(&self, io: &ShellIo) {
        let Some(p) = &self.project else {
            return;
        };
        let project_id = p.id;
        let repo = PathBuf::from(&p.path);
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let (b, d, s) = tokio::task::spawn_blocking(move || {
                (
                    delivery::branch(&repo),
                    delivery::is_dirty(&repo),
                    delivery::file_statuses(&repo),
                )
            })
            .await
            .unwrap_or((None, false, HashMap::new()));
            let _ = proxy.send_event(Message::ProjectGitRefreshed(project_id, b, d, s));
        });
    }
```

改成:

```rust
    /// 异步刷新当前项目的 git 分支/脏/文件状态/worktree 列表(打开项目 +
    /// 回合结束 + `git_watch` 检测到变化时触发,见 `Message::ProjectFsChanged`)。
    fn spawn_project_git_refresh(&self, io: &ShellIo) {
        let Some(p) = &self.project else {
            return;
        };
        let project_id = p.id;
        let repo = PathBuf::from(&p.path);
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let (b, d, s, w) = tokio::task::spawn_blocking(move || {
                (
                    delivery::branch(&repo),
                    delivery::is_dirty(&repo),
                    delivery::file_statuses(&repo),
                    delivery::worktrees(&repo),
                )
            })
            .await
            .unwrap_or((None, false, HashMap::new(), Vec::new()));
            let _ = proxy.send_event(Message::ProjectGitRefreshed(project_id, b, d, s, w));
        });
    }
```

- [ ] **Step 4: `from_restore` 启动 watcher**

在 1538-1624 行的 `from_restore` 函数里,找到:

```rust
        ws.ensure_project_terminal(io);
        // 认回上次退出前打开的预览文件 tab（重启后自动重开）。
        ws.restore_preview_state();
        ws.spawn_project_git_refresh(io);
        ws.spawn_conversations_refresh(io);
        ws.spawn_acceptance_count_refresh(io);
        ws
    }
```

改成:

```rust
        ws.ensure_project_terminal(io);
        // 认回上次退出前打开的预览文件 tab（重启后自动重开）。
        ws.restore_preview_state();
        ws.spawn_project_git_refresh(io);
        ws.spawn_conversations_refresh(io);
        ws.spawn_acceptance_count_refresh(io);
        ws.start_git_watch(io);
        ws
    }

    /// 启动本项目的实时文件系统监听(D4)。失败(比如 fd 耗尽)只记一条
    /// warn,不影响项目正常打开——退化成"只在开项目/回合结束时刷新"这个
    /// D4 之前就有的行为。
    fn start_git_watch(&mut self, io: &ShellIo) {
        let Some(p) = &self.project else {
            return;
        };
        let project_id = p.id;
        let repo = PathBuf::from(&p.path);
        let proxy = io.proxy.clone();
        match git_watch::start(&io.handle, repo, std::time::Duration::from_millis(300), move |relevance| {
            let _ = proxy.send_event(Message::ProjectFsChanged(project_id, relevance));
        }) {
            Ok(handle) => self.git_watch = Some(handle),
            Err(err) => tracing::warn!(project_id, %err, "git_watch 启动失败,降级为手动刷新"),
        }
    }
```

- [ ] **Step 5: 消息处理分支**

把 3785-3791 行:

```rust
            Message::ProjectGitRefreshed(project_id, branch, dirty, statuses) => {
                self.with_project(project_id, move |ws, _io| {
                    ws.branch = branch;
                    ws.dirty = dirty;
                    ws.git_statuses = statuses;
                });
            }
```

改成:

```rust
            Message::ProjectGitRefreshed(project_id, branch, dirty, statuses, worktrees) => {
                self.with_project(project_id, move |ws, _io| {
                    ws.branch = branch;
                    ws.dirty = dirty;
                    ws.git_statuses = statuses;
                    ws.worktrees = worktrees;
                });
            }
            Message::ProjectFsChanged(project_id, _relevance) => {
                // Plan 2 会在这里按 `_relevance == Relevance::GitRefs` 加一段
                // Git Log 快照重建;这里先只管文件树/worktree 刷新。
                self.with_project(project_id, |ws, io| {
                    ws.spawn_project_git_refresh(io);
                });
            }
```

（`with_project` 签名是 `fn with_project(&mut self, project_id: ProjectId, f: impl FnOnce(&mut Workspace, &ShellIo))`——`workspace.rs:2645`,闭包能拿到 `&ShellIo`,上面两处用法直接匹配这个签名,不用改 `with_project` 本身。）

- [ ] **Step 6: 跑 `cargo build` 确认这一层能编译**

Run: `cargo build -p dozer-app 2>&1 | head -80`
Expected: 剩下的编译错误应该只集中在 Task 7 要改的树行渲染代码(`tree_row_dot`/6250-6312 行区间)和它的测试(8672-8674)。如果这一步之外还有其他文件报错,先看清楚是不是本任务漏改的地方,不要跳过。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): Workspace 接入 worktree 列表与 git_watch 实时刷新

ProjectGitRefreshed 消息带上 worktree 列表;新增 ProjectFsChanged,
由 git_watch 的 debounce 回调触发,复用既有 spawn_project_git_refresh
管线。watcher 生命周期挂在 Workspace 字段上,RAII 自动停。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

（此时整个 crate 大概率仍编译不过,卡在 Task 7 要改的文件树渲染代码——正常,继续。）

---

### Task 7: 文件树行渲染——填充/空心色点 + 既有测试更新

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:6250-6254`(`dir_status`/`git_statuses.get` 调用)
- Modify: `crates/dozer-app/src/workspace.rs:6306-6312`(色点渲染)
- Modify: `crates/dozer-app/src/workspace.rs:7383-7389`(`tree_row_dot`)
- Modify: `crates/dozer-app/src/workspace.rs:8672-8674`(既有测试)

**Interfaces:**
- Consumes: `delivery::{ChangeKind, FileGitStatus, DirGitStatus}`(Task 1/3)。
- Produces: `fn tree_row_dot_color(kind: ChangeKind) -> Color`、`fn tree_row_dot_glyph(unstaged: bool) -> &'static str`(替代旧的单一 `tree_row_dot`,拆成颜色/字形两个纯函数,方便分别单测)。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/workspace.rs` 的测试模块里,找到既有的(8672-8674 行附近):

```rust
        assert_eq!(tree_row_dot(FileStatus::Modified), theme::GOLD);
        assert_eq!(tree_row_dot(FileStatus::New), theme::GREEN);
        assert_eq!(tree_row_dot(FileStatus::Deleted), theme::RED);
```

先看清楚它所在的完整 `#[test] fn ...() { ... }`(用 `Read` 工具看这一段前后 10 行,确认函数名和是否还有其他断言),把这三行替换成:

```rust
        assert_eq!(tree_row_dot_color(ChangeKind::Modified), theme::GOLD);
        assert_eq!(tree_row_dot_color(ChangeKind::New), theme::GREEN);
        assert_eq!(tree_row_dot_color(ChangeKind::Deleted), theme::RED);
        assert_eq!(tree_row_dot_glyph(false), "●", "全部暂存=实心");
        assert_eq!(tree_row_dot_glyph(true), "○", "有未暂存改动=空心");
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --lib -- tree_row_dot 2>&1 | head -40`
Expected: 编译失败(`tree_row_dot_color`/`tree_row_dot_glyph` 不存在,`tree_row_dot`/`FileStatus` 也已经在别处不存在了)。

- [ ] **Step 3: 重写 `tree_row_dot`**

第 7383-7389 行左右:

```rust
fn tree_row_dot(status: FileStatus) -> Color {
    match status {
        FileStatus::Modified => theme::GOLD,
        FileStatus::New => theme::GREEN,
        FileStatus::Deleted => theme::RED,
    }
}
```

改成:

```rust
/// 色点颜色编码改动类型(kind),不变——D2 只新增了填充态维度,不推翻既有
/// 配色约定。
fn tree_row_dot_color(kind: delivery::ChangeKind) -> Color {
    match kind {
        delivery::ChangeKind::Modified => theme::GOLD,
        delivery::ChangeKind::New => theme::GREEN,
        delivery::ChangeKind::Deleted => theme::RED,
    }
}

/// 色点字形编码暂存态(D2):全部暂存(无未暂存改动)→ 实心 `●`;有任何未
/// 暂存改动(不论是否同时有暂存部分)→ 空心 `○`。尾缀字符不重复编码 kind
/// (颜色已经够用),避免过度设计。
fn tree_row_dot_glyph(unstaged: bool) -> &'static str {
    if unstaged { "○" } else { "●" }
}
```

- [ ] **Step 4: 更新树行渲染调用点**

第 6250-6254 行:

```rust
                    let status = if row.is_dir {
                        delivery::dir_status(&row.path, &ws.git_statuses)
                    } else {
                        ws.git_statuses.get(&row.path).copied()
                    };
```

这两条分支现在返回的类型不一样(`DirGitStatus` vs `FileGitStatus`),但下游只需要 `(kind, unstaged)` 这一对,所以统一成:

```rust
                    let status: Option<(delivery::ChangeKind, bool)> = if row.is_dir {
                        delivery::dir_status(&row.path, &ws.git_statuses)
                            .map(|d| (d.kind, d.unstaged))
                    } else {
                        ws.git_statuses.get(&row.path).map(|s| (s.kind, s.unstaged))
                    };
```

第 6306-6312 行:

```rust
                    if let Some(st) = status {
                        line = line.push(iced_widget::space::horizontal());
                        line = line.push(
                            text("●")
                                .size(workspace_font::dot_xs())
                                .color(tree_row_dot(st)),
                        );
                    }
```

改成:

```rust
                    if let Some((kind, unstaged)) = status {
                        line = line.push(iced_widget::space::horizontal());
                        line = line.push(
                            text(tree_row_dot_glyph(unstaged))
                                .size(workspace_font::dot_xs())
                                .color(tree_row_dot_color(kind)),
                        );
                    }
```

- [ ] **Step 5: 全量编译 + 测试**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 干净编译,0 错误。如果还有报错,大概率是某个测试文件(`cargo test` 构建的那份)里还有别的地方引用了旧 `FileStatus`/`tree_row_dot`——用 `grep -rn "FileStatus\b" crates/dozer-app/src/` 找出来,按同样的模式(`ChangeKind`/`FileGitStatus`)改掉,这份计划前面没列到的话说明是遗漏,照着已经改过的地方的思路处理,不要引入新的抽象。

Run: `cargo test -p dozer-app 2>&1 | tail -40`
Expected: 全部测试(含本计划新增的)PASS,无一失败。

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings`
Expected: 干净。

Run: `cargo fmt -p dozer-app -- --check`
Expected: 干净;若有格式问题,跑 `cargo fmt -p dozer-app` 后重新 `git add`。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): 文件树色点区分暂存/工作区(实心/空心)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: 手动验收 + 收尾

**Files:** 无代码改动,纯验证。

- [ ] **Step 1: 全量检查**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets -- -D warnings
cargo fmt -p dozer-app -- --check
```

Expected: 四条全部干净通过。

- [ ] **Step 2: 手动验收(用 `run` 技能或直接 `cargo run -p dozer-app`)**

1. 打开一个真实 git 项目(比如 dozer 自己)。
2. `git add` 一个已跟踪文件的部分改动,不 commit → 文件树对应行应显示**实心**金点。
3. 在这基础上再不经 `git add` 直接改一次同一个文件 → 应变成**空心**金点(部分暂存+未暂存)。
4. 新建一个未跟踪文件,不 `git add` → 应显示**空心**绿点。
5. 不经过 Dozer,在外部终端对这个仓库 `git worktree add ../wt-test -b test-branch` → 几秒内(debounce 300ms + 下次某个文件系统事件触发)不需要手动操作,Dozer 侧下次任何 git 相关刷新(比如再碰一下某个文件)后,后续 Plan 2 的 Git Log 面板会能看到这个新 worktree(本计划本身不产出 UI 展示 worktree 列表的地方,只产出数据——`ws.worktrees` 字段有值即算这一步通过,可以临时加一行 `tracing::info!(?ws.worktrees)` 或用调试器/日志确认,验收完记得删掉临时日志)。
6. 外部终端 `git commit` 一次 → 文件树装饰应在亚秒级消失(不需要手动切换项目页签或等回合结束)。

- [ ] **Step 3: 更新 CLAUDE.md 或相关文档(如有必要)**

检查 `crates/dozer-app/Cargo.toml` 里 `gleisbau` 依赖旁边 Task 1 留的那条"spike(2026-08-06)"注释(见 `git_log.rs` 引入时加的),如果这份计划执行完之后 `git2` 已经是稳定的直接依赖(不再是"spike 验证中"的状态),可以把那条注释里"若验证通过转正"的措辞更新掉,反映"已转正"的现状。不是强制项,但顺手做了更准确。

- [ ] **Step 4: 最终 Commit(如果 Step 3 有改动)**

```bash
git add crates/dozer-app/Cargo.toml
git commit -m "$(cat <<'EOF'
docs(dozer-app): 更新 gleisbau/git2 依赖注释,反映 spike 已转正

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## 完成后的状态

- `delivery.rs` 的 git 读路径全部基于 `git2`,不再 shell 出 `git status`/`git branch` 相关命令(写路径 `accept`/`update-ref` 不变)。
- 文件树能区分暂存(实心)/工作区(空心)改动,数据实时(debounce ~300ms)刷新,不用等开项目/回合结束。
- `Workspace.worktrees` 字段随每次 git 刷新一起更新,包含同仓库全部 worktree(主 + 链接)的路径/分支/脏/是否当前/是否目录已丢失。
- **本计划不产出任何展示 worktree 列表或 Git Log 图的 UI**——那是 `docs/superpowers/plans/2026-08-06-dozer-git-log-panel-graduation.md`(Plan 2)的范围,Plan 2 直接消费本计划产出的 `delivery::WorktreeInfo`/`Message::ProjectFsChanged`。
