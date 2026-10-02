# bytegit P0:仓库骨架与基础类型 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立独立的 `bytegit` 库骨架,交付基础类型(`CommitId`/`BlobId`/`ChangeKind`/`GitError`)、`Repo::discover` 与确定性测试夹具 `TempRepo`,发 `v0.1.0`。

**Architecture:** 单 crate 库,`Repo` 句柄包着私有的 `git2::Repository`,公开 API 不出现 `git2` 类型;测试夹具用 git2 构建固定作者/时间的临时仓库,供本库与下游面板测试共用。本阶段不迁移 dozer 任何代码(P1 起才让 `dozer-app` 依赖它)。

**Tech Stack:** Rust edition 2024、`git2 = "0.21"`、`serde`、`tempfile`(feature `testutil`)。

**Spec:** `docs/superpowers/specs/2026-10-02-bytegit-design.md`(§2 仓库与形态、§3 设计原则、§4.1、§4.7、§5、§6 的 P0)。背景:`docs/dozer-v2/bytegit-调用点盘点.md`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`。

**本计划中的代码已在草稿目录里完整跑过**:`cargo test`(默认与 `--all-features`)28 个测试通过,`cargo clippy --all-targets -- -D warnings`(默认与 `--all-features`)与 `cargo fmt --check` 干净。执行者照抄文件内容即可;若结果与此不符,先停下排查,不要改测试迎合实现。

## Global Constraints

- Rust edition `2024`,许可证 `MIT`(与 byteui 一致),仓库地址 `https://github.com/byteboyai/bytegit`。
- `git2 = "0.21"`:必须与 dozer 的 `dozer-app/Cargo.toml` 及 `gleisbau` 传递依赖对齐,避免 cargo 拉两份。
- 公开 API 不得出现 `git2::*` 类型;`git2` 只出现在私有字段和 `pub(crate)` 方法里。
- 全部同步;不依赖 `tokio`、`iced`、`notify`(`watch` feature 属于 P4,本阶段 `Cargo.toml` 不声明它)。
- 消费方一律用 git tag 引用(`bytegit = { git = "...", tag = "vX.Y.Z" }`),本地联调用不提交的 `[patch]`。
- 提交信息用英文前缀风格(`feat:`/`chore:`/`docs:`/`test:`),与 dozer、byteui 一致;提交末尾带仓库要求的 `Co-Authored-By` 行。
- **git2 0.21 的 API 与旧版不同**:`Reference::shorthand()`、`Remote::url()` 返回 `Result<&str, Error>` 而不是 `Option<&str>`(P1 起迁移 `delivery.rs` 时会碰到)。

## Review Focus

规格对下列输入没有逐条说明,但真实使用者会遇到;每条都由下面指明的任务里的测试固定下来(行为均已在草稿里实测确认,不是假设):

1. **`discover` 传入不存在的路径** → 应报 `Io`(路径不存在),而不是 libgit2 默认的 `RefNotFound` 误导性分类。Task 2 `discover_nonexistent_path_is_io_not_ref_not_found`。
2. **`discover` 传入文件路径、`.git` 目录内部路径** → 应找到仓库并返回工作区根。Task 2 `discover_from_a_file_path_finds_the_repo`、`discover_from_inside_dot_git_returns_the_workdir`。
3. **bare 仓库** → `is_bare()` 为真,`root()` 退为 git 目录,不 panic。Task 2 `bare_repo_reports_bare_and_uses_git_dir_as_root`。
4. **初始分支不叫 master(如 `main`)的空仓库、detached HEAD** → `is_empty` 必须分别为 `true`/`false`;`git2::Repository::is_empty` 对 `main` 会误报非空,所以不能直接用。Task 2 `fresh_repo_is_empty_until_first_commit`、`empty_repo_with_default_branch_name_is_empty`、`detached_head_is_not_empty`。
5. **`root()` 的形态**:libgit2 返回带尾部分隔符且已解析符号链接的路径(macOS `/var` → `/private/var`)。尾部分隔符要去掉,符号链接解析要写进文档提醒调用方先规范化再比较。Task 2 `root_has_no_trailing_separator`;另 `CommitId` 解析畸形十六进制要报错而不是 panic(Task 1 `rejects_malformed_hex`)。

---

## File Structure

新仓库 `~/Projects/CoralProjects/byteboy/bytegit/`(与 `byteui`、`dozer` 并列):

| 文件 | 职责 |
|------|------|
| `Cargo.toml` | 包元数据、依赖、`testutil` feature |
| `LICENSE` | MIT,从 byteui 复制 |
| `.gitignore` | `/target` |
| `README.md` | 一页说明:是什么、不是什么、如何依赖、本地联调 |
| `.github/workflows/ci.yml` | fmt + clippy + test,默认与 `--all-features` 各跑一遍 |
| `src/lib.rs` | 模块声明与公开再导出 |
| `src/error.rs` | `GitError`、`GitErrorKind`、`From<git2::Error>`、`From<io::Error>` |
| `src/id.rs` | `CommitId`、`BlobId`(宏生成,内部包 `git2::Oid`) |
| `src/change.rs` | `ChangeKind` 及内部的 `Delta` 转换 |
| `src/repo.rs` | `Repo`:`discover`、`root`、`is_bare`、`is_empty` |
| `src/testutil.rs` | `TempRepo` 夹具(`cfg(any(test, feature = "testutil"))`) |

dozer 仓库在 Task 3 里改两份文档(规格与要求文档),不改任何代码。

---

### Task 1: 仓库骨架与基础类型

**Files:**
- Create: `~/Projects/CoralProjects/byteboy/bytegit/{Cargo.toml,LICENSE,.gitignore,README.md,.github/workflows/ci.yml}`
- Create: `src/lib.rs`、`src/error.rs`、`src/id.rs`、`src/change.rs`

**Interfaces:**
- Produces(Task 2 依赖):
  - `GitError::new(kind: GitErrorKind, message: impl Into<String>) -> GitError`、`GitError::kind(&self) -> GitErrorKind`、`GitError::message(&self) -> &str`;`impl From<git2::Error> for GitError`、`impl From<std::io::Error> for GitError`
  - `GitErrorKind::{NotARepo, RefNotFound, NoCommits, GitBinaryUnavailable, Io, Backend}`(`Copy + PartialEq + Debug`)
  - `CommitId`/`BlobId`:`Copy + Eq + Hash + Ord + Display + Debug + FromStr<Err = GitError> + Serialize + Deserialize`;方法 `to_hex(self) -> String`、`short(self, n: usize) -> String`;`pub(crate)` 的 `from_oid(git2::Oid) -> Self`、`oid(self) -> git2::Oid`
  - `ChangeKind::{Added, Modified, Deleted, Renamed, Copied, TypeChange}`;`pub(crate) fn from_delta(git2::Delta) -> Option<ChangeKind>`

- [ ] **Step 1: 创建仓库目录并初始化 git**

```bash
mkdir -p ~/Projects/CoralProjects/byteboy/bytegit/src ~/Projects/CoralProjects/byteboy/bytegit/.github/workflows
cd ~/Projects/CoralProjects/byteboy/bytegit
git init -b main
cp ~/Projects/CoralProjects/byteboy/byteui/LICENSE LICENSE
printf '/target\n' > .gitignore
```

Expected: `Initialized empty Git repository`;若目录已存在且非空,先停下问用户。

- [ ] **Step 2: 写 `Cargo.toml`**

```toml
[package]
name = "bytegit"
version = "0.1.0"
edition = "2024"
license = "MIT"
description = "ByteBoy 系产品共用的本地 Git 底层库"
repository = "https://github.com/byteboyai/bytegit"

[features]
# 测试夹具:`TempRepo` 用 git2 构建确定性的临时仓库。下游面板的单元测试也用它。
testutil = ["dep:tempfile"]

[dependencies]
git2 = "0.21"
serde = { version = "1", features = ["derive"] }
tempfile = { version = "3", optional = true }

[dev-dependencies]
tempfile = "3"
serde_json = "1"
```

- [ ] **Step 3: 写 `src/lib.rs`**(此时 `repo`/`testutil` 还不存在,先只声明本任务的模块)

```rust
//! bytegit:ByteBoy 系产品共用的本地 Git 底层。
//!
//! 全部同步;公开 API 不暴露 `git2` 类型。设计见 dozer 仓库
//! `docs/superpowers/specs/2026-10-02-bytegit-design.md`。

mod change;
mod error;
mod id;

pub use change::ChangeKind;
pub use error::{GitError, GitErrorKind};
pub use id::{BlobId, CommitId};
```

- [ ] **Step 4: 写 `src/error.rs`(含测试)**

```rust
use std::fmt;

/// 稳定的错误分类,调用方按它分支;展示用 [`GitError::message`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitErrorKind {
    /// 路径不在任何 git 仓库内。
    NotARepo,
    /// 引用(分支/提交/路径)不存在。
    RefNotFound,
    /// 仓库还没有任何提交。
    NoCommits,
    /// 需要 `git` 可执行文件但机器上没有(仅写操作的命令行实现会用到)。
    GitBinaryUnavailable,
    Io,
    /// 其余后端错误。
    Backend,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError {
    kind: GitErrorKind,
    message: String,
}

impl GitError {
    pub fn new(kind: GitErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> GitErrorKind {
        self.kind
    }

    /// 适合直接展示给用户的文本。
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GitError {}

impl From<git2::Error> for GitError {
    fn from(e: git2::Error) -> Self {
        let kind = match (e.code(), e.class()) {
            (git2::ErrorCode::NotFound, git2::ErrorClass::Repository) => GitErrorKind::NotARepo,
            (git2::ErrorCode::UnbornBranch, _) => GitErrorKind::NoCommits,
            (git2::ErrorCode::NotFound, _) => GitErrorKind::RefNotFound,
            _ => GitErrorKind::Backend,
        };
        Self::new(kind, e.message())
    }
}

impl From<std::io::Error> for GitError {
    fn from(e: std::io::Error) -> Self {
        Self::new(GitErrorKind::Io, e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_maps_to_io_kind() {
        let e: GitError = std::io::Error::other("boom").into();
        assert_eq!(e.kind(), GitErrorKind::Io);
        assert_eq!(e.message(), "boom");
        assert_eq!(e.to_string(), "boom");
    }

    #[test]
    fn git2_not_found_in_repository_class_is_not_a_repo() {
        let e = git2::Error::new(
            git2::ErrorCode::NotFound,
            git2::ErrorClass::Repository,
            "could not find repository",
        );
        assert_eq!(GitError::from(e).kind(), GitErrorKind::NotARepo);
    }

    #[test]
    fn git2_not_found_elsewhere_is_ref_not_found() {
        let e = git2::Error::new(
            git2::ErrorCode::NotFound,
            git2::ErrorClass::Reference,
            "no such ref",
        );
        assert_eq!(GitError::from(e).kind(), GitErrorKind::RefNotFound);
    }

    #[test]
    fn git2_unborn_branch_is_no_commits() {
        let e = git2::Error::new(
            git2::ErrorCode::UnbornBranch,
            git2::ErrorClass::Reference,
            "unborn",
        );
        assert_eq!(GitError::from(e).kind(), GitErrorKind::NoCommits);
    }
}
```

- [ ] **Step 5: 写 `src/id.rs`(含测试)**

```rust
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{GitError, GitErrorKind};

macro_rules! object_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(git2::Oid);

        impl $name {
            // 后续阶段(log/diff)才会在非测试代码里用到这两个转换。
            #[allow(dead_code)]
            pub(crate) fn from_oid(oid: git2::Oid) -> Self {
                Self(oid)
            }

            #[allow(dead_code)]
            pub(crate) fn oid(self) -> git2::Oid {
                self.0
            }

            /// 完整的十六进制 id。
            pub fn to_hex(self) -> String {
                self.0.to_string()
            }

            /// 前 `n` 位十六进制(`n` 超过长度时返回完整 id)。
            pub fn short(self, n: usize) -> String {
                let full = self.0.to_string();
                full[..n.min(full.len())].to_string()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl FromStr for $name {
            type Err = GitError;

            fn from_str(s: &str) -> Result<Self, GitError> {
                if s.len() != 40 {
                    return Err(GitError::new(
                        GitErrorKind::Backend,
                        format!("不是 40 位十六进制 id: {s}"),
                    ));
                }
                git2::Oid::from_str(s).map(Self).map_err(GitError::from)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

object_id!(
    /// 提交 id。对外不暴露 `git2::Oid`。
    CommitId
);
object_id!(
    /// 文件内容(blob)id。
    BlobId
);

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn parses_and_displays_round_trip() {
        let id: CommitId = HEX.parse().unwrap();
        assert_eq!(id.to_hex(), HEX);
        assert_eq!(id.to_string(), HEX);
    }

    #[test]
    fn short_truncates_and_clamps() {
        let id: CommitId = HEX.parse().unwrap();
        assert_eq!(id.short(7), "0123456");
        assert_eq!(id.short(100), HEX);
        assert_eq!(id.short(0), "");
    }

    #[test]
    fn rejects_malformed_hex() {
        assert!("abc".parse::<CommitId>().is_err());
        assert!(
            "zz23456789abcdef0123456789abcdef01234567"
                .parse::<CommitId>()
                .is_err()
        );
    }

    #[test]
    fn serde_is_a_plain_hex_string() {
        let id: CommitId = HEX.parse().unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{HEX}\""));
        let back: CommitId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn debug_names_the_type() {
        let id: BlobId = HEX.parse().unwrap();
        assert_eq!(format!("{id:?}"), format!("BlobId({HEX})"));
    }
}
```

- [ ] **Step 6: 写 `src/change.rs`(含测试)**

```rust
/// 一次变更的种类。替换调用方现在直接用的 `git2::Delta`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChange,
}

impl ChangeKind {
    /// `Unmodified`/`Ignored`/`Untracked`/`Unreadable`/`Conflicted` 不是"变更",返回 `None`。
    // 后续阶段(commit_files/status)才会在非测试代码里用到。
    #[allow(dead_code)]
    pub(crate) fn from_delta(delta: git2::Delta) -> Option<Self> {
        match delta {
            git2::Delta::Added => Some(Self::Added),
            git2::Delta::Modified => Some(Self::Modified),
            git2::Delta::Deleted => Some(Self::Deleted),
            git2::Delta::Renamed => Some(Self::Renamed),
            git2::Delta::Copied => Some(Self::Copied),
            git2::Delta::Typechange => Some(Self::TypeChange),
            git2::Delta::Unmodified
            | git2::Delta::Ignored
            | git2::Delta::Untracked
            | git2::Delta::Unreadable
            | git2::Delta::Conflicted => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_real_changes() {
        assert_eq!(
            ChangeKind::from_delta(git2::Delta::Added),
            Some(ChangeKind::Added)
        );
        assert_eq!(
            ChangeKind::from_delta(git2::Delta::Modified),
            Some(ChangeKind::Modified)
        );
        assert_eq!(
            ChangeKind::from_delta(git2::Delta::Deleted),
            Some(ChangeKind::Deleted)
        );
        assert_eq!(
            ChangeKind::from_delta(git2::Delta::Renamed),
            Some(ChangeKind::Renamed)
        );
        assert_eq!(
            ChangeKind::from_delta(git2::Delta::Copied),
            Some(ChangeKind::Copied)
        );
        assert_eq!(
            ChangeKind::from_delta(git2::Delta::Typechange),
            Some(ChangeKind::TypeChange)
        );
    }

    #[test]
    fn non_changes_map_to_none() {
        for d in [
            git2::Delta::Unmodified,
            git2::Delta::Ignored,
            git2::Delta::Untracked,
            git2::Delta::Unreadable,
            git2::Delta::Conflicted,
        ] {
            assert_eq!(ChangeKind::from_delta(d), None, "{d:?}");
        }
    }
}
```

- [ ] **Step 7: 运行测试**

Run: `cargo fmt && cargo test`
Expected: 11 个测试通过(`error` 4、`id` 5、`change` 2),全部 `ok`,无警告。

- [ ] **Step 8: 写 CI 与 README**

`.github/workflows/ci.yml`(在 byteui 的基础上多跑 `--all-features`,因为 `testutil` 代码只在该 feature 下编译进库):

```yaml
name: ci
on:
  push:
    branches: [main]
  pull_request:
jobs:
  test:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo clippy --all-targets --all-features -- -D warnings
      - run: cargo test
      - run: cargo test --all-features
```

`README.md`:

````markdown
# bytegit

ByteBoy 系产品(Dozer、Digger……)共用的**本地 Git 底层库**。

- 全部同步 API,不依赖 tokio / iced。
- 公开类型不暴露 `git2`,调用方不被 `git2` 版本锁死。
- 不含托管平台账户、提交图布局、worktree/commit/merge(见设计规格)。

## 依赖

```toml
bytegit = { git = "https://github.com/byteboyai/bytegit", tag = "v0.1.0" }
# 下游测试里要用确定性临时仓库夹具:
# bytegit = { git = "...", tag = "v0.1.0", features = ["testutil"] }
```

一律用 tag。本地联调在使用方的 `.cargo/config.toml` 里(不提交):

```toml
[patch."https://github.com/byteboyai/bytegit"]
bytegit = { path = "../bytegit" }
```

## 设计

dozer 仓库 `docs/superpowers/specs/2026-10-02-bytegit-design.md`。
````

- [ ] **Step 9: 门禁**

Run: `cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --check`
Expected: 无输出、退出码 0。

- [ ] **Step 10: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git add -A
git status --short   # 应只有上面列出的文件,没有 target/
git commit -m "feat: scaffold bytegit with GitError, CommitId/BlobId, ChangeKind

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `Repo::discover` 与 `TempRepo` 夹具

**Files:**
- Create: `src/repo.rs`、`src/testutil.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: Task 1 的 `GitError`、`GitErrorKind`、`CommitId::from_oid`、`CommitId::oid`
- Produces(P1 起所有阶段依赖):
  - `Repo::discover(path: &Path) -> Result<Repo, GitError>`、`Repo::root(&self) -> &Path`、`Repo::is_bare(&self) -> bool`、`Repo::is_empty(&self) -> Result<bool, GitError>`;`pub(crate) fn raw(&self) -> &git2::Repository`;`Repo: Send`(不是 `Sync`)
  - `testutil::TempRepo`:`new()`、`path() -> &Path`、`open() -> Repo`、`write_untracked(rel, content) -> &Self`、`commit_file(rel, content, message) -> CommitId`、`branch(name) -> &Self`、`checkout(name) -> &Self`、`add_remote(name, url) -> &Self`;所有操作确定性(固定作者与时间,同样的操作序列得到同样的提交 id)

- [ ] **Step 1: 写 `src/testutil.rs`(含测试)**

```rust
//! 测试夹具:用 git2 构建确定性的临时仓库,不调用命令行 `git`。
//!
//! 作者、邮箱与提交时间固定(第 n 次提交时间 = 基准 + n 分钟),
//! 所以同样的操作序列在任何机器上得到同样的提交 id。

use std::cell::Cell;
use std::path::Path;

use crate::{CommitId, Repo};

const BASE_TIME: i64 = 1_700_000_000;

pub struct TempRepo {
    dir: tempfile::TempDir,
    repo: git2::Repository,
    commits: Cell<i64>,
}

impl Default for TempRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl TempRepo {
    /// 空仓库,初始分支 `main`。
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let repo = git2::Repository::init(dir.path()).expect("git init 失败");
        repo.set_head("refs/heads/main").expect("设置初始分支失败");
        Self {
            dir,
            repo,
            commits: Cell::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn raw_repo(&self) -> &git2::Repository {
        &self.repo
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// 以 bytegit 的 `Repo` 打开(每次新开一个句柄)。
    pub fn open(&self) -> Repo {
        Repo::discover(self.path()).expect("打开临时仓库失败")
    }

    fn signature(&self) -> git2::Signature<'static> {
        let n = self.commits.get();
        git2::Signature::new(
            "Test",
            "test@example.com",
            &git2::Time::new(BASE_TIME + n * 60, 0),
        )
        .expect("构造签名失败")
    }

    /// 写入(必要时创建父目录)但不加入暂存区。
    pub fn write_untracked(&self, rel: &str, content: &str) -> &Self {
        let full = self.path().join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("创建目录失败");
        }
        std::fs::write(full, content).expect("写文件失败");
        self
    }

    /// 写文件、暂存并提交到当前分支。
    pub fn commit_file(&self, rel: &str, content: &str, message: &str) -> CommitId {
        self.write_untracked(rel, content);
        let mut index = self.repo.index().expect("读取 index 失败");
        index.add_path(Path::new(rel)).expect("暂存失败");
        index.write().expect("写 index 失败");
        let tree = self
            .repo
            .find_tree(index.write_tree().expect("写 tree 失败"))
            .expect("找不到 tree");
        let sig = self.signature();
        let parents: Vec<git2::Commit> = match self.repo.head() {
            Ok(head) => vec![head.peel_to_commit().expect("HEAD 不是提交")],
            Err(_) => Vec::new(), // 第一次提交
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        let oid = self
            .repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
            .expect("提交失败");
        self.commits.set(self.commits.get() + 1);
        CommitId::from_oid(oid)
    }

    /// 在当前 HEAD 创建分支(不切换)。
    pub fn branch(&self, name: &str) -> &Self {
        let head = self
            .repo
            .head()
            .expect("创建分支前需要至少一次提交")
            .peel_to_commit()
            .expect("HEAD 不是提交");
        self.repo.branch(name, &head, false).expect("创建分支失败");
        self
    }

    /// 切换到已有分支并更新工作区。
    pub fn checkout(&self, name: &str) -> &Self {
        let refname = format!("refs/heads/{name}");
        let obj = self.repo.revparse_single(&refname).expect("分支不存在");
        self.repo
            .checkout_tree(&obj, Some(git2::build::CheckoutBuilder::new().force()))
            .expect("检出失败");
        self.repo.set_head(&refname).expect("切换 HEAD 失败");
        self
    }

    pub fn add_remote(&self, name: &str, url: &str) -> &Self {
        self.repo.remote(name, url).expect("添加远程失败");
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_operations_give_same_commit_ids() {
        let a = TempRepo::new();
        let b = TempRepo::new();
        let ia = a.commit_file("f.txt", "1", "one");
        let ib = b.commit_file("f.txt", "1", "one");
        assert_eq!(ia, ib);
        let ja = a.commit_file("f.txt", "2", "two");
        let jb = b.commit_file("f.txt", "2", "two");
        assert_eq!(ja, jb);
        assert_ne!(ia, ja);
    }

    #[test]
    fn initial_branch_is_main() {
        let t = TempRepo::new();
        t.commit_file("f.txt", "1", "one");
        let head = t.repo.head().unwrap();
        assert_eq!(head.shorthand().unwrap(), "main");
    }

    #[test]
    fn branch_and_checkout_switch_head_and_files() {
        let t = TempRepo::new();
        t.commit_file("f.txt", "main-content", "one");
        t.branch("feature").checkout("feature");
        t.commit_file("f.txt", "feature-content", "two");
        t.checkout("main");
        let content = std::fs::read_to_string(t.path().join("f.txt")).unwrap();
        assert_eq!(content, "main-content");
        assert_eq!(t.repo.head().unwrap().shorthand().unwrap(), "main");
    }

    #[test]
    fn untracked_file_is_written_but_not_committed() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a", "one");
        t.write_untracked("sub/b.txt", "b");
        assert!(t.path().join("sub/b.txt").exists());
        let tree = t.repo.head().unwrap().peel_to_tree().unwrap();
        assert!(tree.get_path(Path::new("sub/b.txt")).is_err());
    }

    #[test]
    fn add_remote_is_visible_to_git2() {
        let t = TempRepo::new();
        t.add_remote("origin", "https://example.com/x.git");
        let remote = t.repo.find_remote("origin").unwrap();
        assert_eq!(remote.url().unwrap(), "https://example.com/x.git");
    }
}
```

- [ ] **Step 2: 写 `src/repo.rs`(含测试)**

```rust
use std::path::{Path, PathBuf};

use crate::{GitError, GitErrorKind};

/// 打开的仓库句柄。所有操作都是它的方法;`git2::Repository` 不对外暴露。
///
/// `Repo` 是 `Send` 不是 `Sync`:跨线程时各线程自己 `discover`。
pub struct Repo {
    inner: git2::Repository,
    root: PathBuf,
}

impl Repo {
    /// 从 `path` 向上查找仓库,所以项目位于仓库子目录时也能打开。
    ///
    /// 返回的 [`Repo::root`] 是 libgit2 解析后的真实路径(macOS 上 `/var/...`
    /// 会变成 `/private/var/...`),调用方与自己持有的项目路径比较前要先规范化。
    pub fn discover(path: &Path) -> Result<Self, GitError> {
        // libgit2 对不存在的路径报 NotFound,会被误归为 RefNotFound;这里先挡掉。
        if !path.exists() {
            return Err(GitError::new(
                GitErrorKind::Io,
                format!("路径不存在: {}", path.display()),
            ));
        }
        let inner = git2::Repository::discover(path)?;
        let dir = match inner.workdir() {
            Some(dir) => dir,
            // bare 仓库没有工作区,退而用 git 目录本身。
            None => inner.path(),
        };
        // libgit2 返回的目录带尾部分隔符,统一去掉。
        let root: PathBuf = dir.components().collect();
        Ok(Self { inner, root })
    }

    /// 工作区根目录(bare 仓库为 git 目录)。
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn is_bare(&self) -> bool {
        self.inner.is_bare()
    }

    /// 还没有任何提交(HEAD 指向尚未诞生的分支)。
    ///
    /// 不用 `git2::Repository::is_empty`:它只认默认分支名为 master 的空仓库,
    /// 初始分支叫 `main` 时会误报"非空"。
    pub fn is_empty(&self) -> Result<bool, GitError> {
        match self.inner.head() {
            Ok(_) => Ok(false),
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => Ok(true),
            Err(e) => Err(GitError::from(e)),
        }
    }

    #[allow(dead_code)] // 后续阶段的方法使用
    pub(crate) fn raw(&self) -> &git2::Repository {
        &self.inner
    }
}

impl std::fmt::Debug for Repo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repo").field("root", &self.root).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempRepo;

    fn same(a: &Path, b: &Path) -> bool {
        a.canonicalize().unwrap() == b.canonicalize().unwrap()
    }

    #[test]
    fn repo_is_send() {
        fn is_send<T: Send>() {}
        is_send::<Repo>();
    }

    #[test]
    fn discover_finds_repo_at_root() {
        let t = TempRepo::new();
        let repo = Repo::discover(t.path()).unwrap();
        assert!(same(repo.root(), t.path()));
        assert!(!repo.is_bare());
    }

    #[test]
    fn discover_works_from_a_subdirectory() {
        let t = TempRepo::new();
        t.write_untracked("a/b/c.txt", "x");
        let repo = Repo::discover(&t.path().join("a/b")).unwrap();
        assert!(same(repo.root(), t.path()));
    }

    #[test]
    fn discover_outside_any_repo_is_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        let err = Repo::discover(dir.path()).unwrap_err();
        assert_eq!(err.kind(), GitErrorKind::NotARepo);
    }

    #[test]
    fn fresh_repo_is_empty_until_first_commit() {
        let t = TempRepo::new();
        assert!(Repo::discover(t.path()).unwrap().is_empty().unwrap());
        t.commit_file("a.txt", "hi", "first");
        assert!(!Repo::discover(t.path()).unwrap().is_empty().unwrap());
    }

    #[test]
    fn root_has_no_trailing_separator() {
        let t = TempRepo::new();
        let repo = Repo::discover(t.path()).unwrap();
        assert!(!repo.root().to_string_lossy().ends_with('/'));
        assert_eq!(repo.root().file_name(), t.path().file_name());
    }

    #[test]
    fn discover_nonexistent_path_is_io_not_ref_not_found() {
        let t = TempRepo::new();
        let err = Repo::discover(&t.path().join("nope/deeper")).unwrap_err();
        assert_eq!(err.kind(), GitErrorKind::Io);
    }

    #[test]
    fn discover_from_a_file_path_finds_the_repo() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "x", "one");
        let repo = Repo::discover(&t.path().join("a.txt")).unwrap();
        assert!(same(repo.root(), t.path()));
    }

    #[test]
    fn discover_from_inside_dot_git_returns_the_workdir() {
        let t = TempRepo::new();
        let repo = Repo::discover(&t.path().join(".git")).unwrap();
        assert!(same(repo.root(), t.path()));
        assert!(!repo.is_bare());
    }

    #[test]
    fn bare_repo_reports_bare_and_uses_git_dir_as_root() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init_bare(dir.path()).unwrap();
        let repo = Repo::discover(dir.path()).unwrap();
        assert!(repo.is_bare());
        assert!(same(repo.root(), dir.path()));
        assert!(repo.is_empty().unwrap());
    }

    #[test]
    fn empty_repo_with_default_branch_name_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        assert!(Repo::discover(dir.path()).unwrap().is_empty().unwrap());
    }

    #[test]
    fn detached_head_is_not_empty() {
        let t = TempRepo::new();
        let id = t.commit_file("a.txt", "x", "one");
        t.raw_repo().set_head_detached(id.oid()).unwrap();
        assert!(!Repo::discover(t.path()).unwrap().is_empty().unwrap());
    }
}
```

- [ ] **Step 3: 更新 `src/lib.rs`**

```rust
//! bytegit:ByteBoy 系产品共用的本地 Git 底层。
//!
//! 全部同步;公开 API 不暴露 `git2` 类型。设计见 dozer 仓库
//! `docs/superpowers/specs/2026-10-02-bytegit-design.md`。

mod change;
mod error;
mod id;
mod repo;
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

pub use change::ChangeKind;
pub use error::{GitError, GitErrorKind};
pub use id::{BlobId, CommitId};
pub use repo::Repo;
```

- [ ] **Step 4: 运行测试,确认全部通过**

Run: `cargo fmt && cargo test && cargo test --all-features`
Expected: 两次都是 `28 passed; 0 failed`。

- [ ] **Step 5: 变异检验(确认 Review Focus 的测试真的会失败)**

把 `repo.rs` 里 `is_empty` 临时改成 `self.inner.is_empty().map_err(GitError::from)`,运行 `cargo test is_empty empty_repo`。
Expected: `fresh_repo_is_empty_until_first_commit` **失败**(`main` 分支的空仓库被误报为非空)。确认后**还原**,再跑 `cargo test` 回到 28 个通过。

再把 `discover` 开头的 `if !path.exists() { ... }` 块临时注释掉,运行 `cargo test nonexistent`。
Expected: `discover_nonexistent_path_is_io_not_ref_not_found` **失败**(得到 `RefNotFound`)。确认后还原。

- [ ] **Step 6: 门禁**

Run: `cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --check`
Expected: 无输出、退出码 0。

- [ ] **Step 7: Commit**

```bash
git add -A
git status --short
git commit -m "feat: add Repo::discover and deterministic TempRepo fixture

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: 发布 v0.1.0 并同步 dozer 文档

**Files:**
- Modify: `~/Projects/CoralProjects/byteboy/dozer/docs/superpowers/specs/2026-10-02-bytegit-design.md`(§3、§4.1、§5)
- Modify: `~/Projects/CoralProjects/byteboy/dozer/docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`(§7)

**Interfaces:** 无新增代码接口。Produces:`bytegit` 的 git tag `v0.1.0`(P1 的 `dozer-app` 依赖它)。

- [ ] **Step 1: 全量门禁(发布前最后一次)**

在 `bytegit` 仓库:

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings && cargo test && cargo test --all-features`
Expected: 全部通过,28 个测试。

- [ ] **Step 2: 向用户确认后再创建远程仓库并推送(对外可见的操作,必须先问)**

在执行前向用户确认:是否在 `byteboyai` 组织下创建 `bytegit` 仓库、可见性是 public 还是 private(byteui 的可见性可作参考:`gh repo view byteboyai/byteui --json visibility`)。获得明确同意后:

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
gh repo create byteboyai/bytegit --<public|private> --source=. --remote=origin --description "ByteBoy 系产品共用的本地 Git 底层库" --push
```

Expected: 仓库创建成功、`main` 已推送。到 GitHub Actions 页确认 `ci` 通过(macOS runner);**CI 红则先修,不打 tag**。

- [ ] **Step 3: 打 tag(同样需用户确认已看过 CI 结果)**

```bash
git tag -a v0.1.0 -m "bytegit v0.1.0: scaffold, discover, TempRepo"
git push origin v0.1.0
```

- [ ] **Step 4: 验证 tag 可被消费**

在草稿目录建一个空 crate 依赖该 tag 并 `cargo check`(不改 dozer):

```bash
cd "$(mktemp -d)" && cargo new --lib consume && cd consume
cargo add bytegit --git https://github.com/byteboyai/bytegit --tag v0.1.0 --features testutil
cargo check
```

Expected: 编译通过。(若仓库是 private,需要本机有访问 byteboyai 的 git 凭据,与 byteui 同理。)

- [ ] **Step 5: 把实测中发现的、与规格不一致之处回写规格**

在 dozer 仓库 `docs/superpowers/specs/2026-10-02-bytegit-design.md`:

1. §3 原则 4 与 §5 `GitError` 一行:改为"`GitError` 是带 `kind()`(`GitErrorKind`,稳定分类)和 `message()` 的结构体"。原文写成 enum 变体,实现选了 struct + kind,满足"稳定 kind + 可展示 message"。
2. §4.1:补三句——`discover` 对不存在的路径返回 `Io`;`root()` 去掉尾部分隔符且是 libgit2 解析后的真实路径(调用方比较前先规范化);`is_empty` 不用 `git2::Repository::is_empty`(它对初始分支非 master 的空仓库误报非空),改为判断 HEAD 是否为未诞生分支。
3. §3 末尾或 §2:补一句"git2 0.21 中 `shorthand()`、`url()` 返回 `Result`"。

- [ ] **Step 6: 在要求文档里标记 P0 完成**

`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` §7 第 1 条后追加一行:`bytegit` P0 已完成(v0.1.0,2026-10-02 之后的日期以实际为准);P1 计划待写。

- [ ] **Step 7: Commit(dozer 仓库)**

提交前 `git status` 与 `git diff --cached --stat` 看全貌——本仓库常有并发会话,只 `git add` 这两份文档。

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git add docs/superpowers/specs/2026-10-02-bytegit-design.md docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md
git diff --cached --stat
git commit -m "docs(bytegit): sync spec with P0 findings, mark P0 done

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage(P0 行:建仓库骨架、CI、`TempRepo`、`CommitId`/`GitError`/`ChangeKind`、`Repo::discover`;发 `v0.1.0`):**
- 仓库骨架/Cargo/LICENSE/README/CI → Task 1;`CommitId`/`BlobId`/`GitError`/`ChangeKind` → Task 1;`Repo::discover`/`root`/`is_bare`/`is_empty` → Task 2;`TempRepo` → Task 2;`v0.1.0` → Task 3。
- 规格 §5 要求 `CommitId` 可序列化 → `id.rs` 的 serde 实现与测试。
- 规格 §4.5 的 `watch` feature、§4.2–4.4/4.6 的 API 不在 P0,属 P1–P5,符合规格 §6。
- 规格与实现的差异(`GitError` 为 struct、`discover` 对不存在路径的行为、`is_empty` 实现)→ Task 3 Step 5 回写。

**Placeholder scan:** 无 TBD/TODO;Task 3 Step 2 的 `<public|private>` 是需用户决定的值,已写明要先问,不是遗漏。

**Type consistency:** `from_oid`/`oid` 在 Task 1 定义、Task 2(`testutil`、`repo` 测试)使用;`GitError::new`、`kind()`、`message()` 全程一致;`TempRepo::open()` 返回 `Repo`,P1 用它。

**Review Focus:** 5 条都对应到已存在的测试,并在 Task 2 Step 5 用变异检验验证其中两条会真正失败。
