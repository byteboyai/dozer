# bytegit P5:写操作迁移(init / clone / checkout)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `delivery.rs` 里仅剩的三个会改仓库的命令行 git 调用(`init_repo`、`clone_repo`、`checkout_branch`)与 `git_available` 搬进 `bytegit`(`bytegit::init`、`bytegit::clone`、`Repo::checkout_branch`、`bytegit::git_available`,发新版本),并给出规格 §4.6 / O2 要求的**评估结论**:哪些能换成 `git2`、哪些必须留在命令行。dozer 侧沿用 P1 的做法,`delivery` 里保留同签名的适配函数(调用点一处不动),行为保持等价。

**Architecture:** 评估用两个可复现的小程序做对比实验(见附录),结论是 **`init` 换成 `git2`,`clone` 与 `checkout_branch` 留在 `bytegit` 内部的命令行实现**——调用方看不到命令行细节,失败时 `message()` 是 git 的 stderr 原文,机器上没有 `git` 返回 `GitErrorKind::GitBinaryUnavailable`。bytegit 的测试把"为什么不能换成 `git2`"钉成会失败的测试(`post-checkout` hook、外部 smudge 过滤器、冲突报错点名文件)。dozer 侧:**先在旧的 `delivery` 函数上补刻画测试**(13 个),再把四个函数的函数体换成 bytegit 调用,同一批测试(加上原有的 clone/`git_available` 测试)在新旧实现上都通过。

**Tech Stack:** Rust edition 2024、`git2 0.21`(`default = []`,**不开** `https`/`ssh`)、标准库 `std::process::Command`;dozer 侧 `bytegit` 以 git tag 依赖。

**Spec:** `docs/superpowers/specs/2026-10-02-bytegit-design.md`(§4.6、§6 的 P5、§8 的 O2)。前序计划:P0–P4。

**本计划中的代码已在草稿里完整跑过:** bytegit 新增 21 个测试,默认构建 125 个全部通过(P3 发布后的基线 104 个 + 21),clippy(默认与 `--all-features`)与 fmt 干净,4 个变异检验都会让对应测试失败(见 Task 1)。dozer 侧在一份克隆里实际编译:刻画测试在**旧实现上** `delivery::` 42 个通过(原有 29 个 + 新增 13 个);切换后同样 42 个通过,`cargo test -p dozer-app` 通过数与旧实现一致,`cargo clippy -p dozer-app --all-targets` 在 `delivery.rs` 上无诊断;一个 dozer 侧变异检验会让 `checkout_from_a_repo_subdirectory_switches_the_whole_repo` 失败。评估实验的原始输出见"评估结论"。执行者照抄即可;结果与此不符先停下排查,不要改测试迎合实现。

## 评估结论(规格 §4.6 / O2)

环境:macOS,git 2.55.0,libgit2 1.9.7(`git2 0.21` 的 `libgit2-sys 0.18.8`),`git2` 不开任何 feature。程序在附录 A、B,**都用隔离的 HOME/配置**,可以重跑。

**`init` → 可以换 `git2`。** 初始分支遵循 `init.defaultBranch`(`trunk` 配置下命令行与 libgit2 都是 `trunk`;没配置都是 `master`);写出的 `config` 与命令行**逐键相同**;对已有仓库重复 `init` 都无害;`init.templateDir` 指向的自定义模板都被应用。差异只有无关紧要的:命令行会复制 14 个 `hooks/*.sample` 示例文件,libgit2 只放一个 `hooks/README.sample`;libgit2 总会建 `info/`。**一个必须手工补齐的差异**:在不存在的目录上,libgit2 会 `mkdir -p` 并成功,命令行(在不存在的工作目录里)根本跑不起来——`bytegit::init` 显式要求目录已存在,保持旧行为。
  - 实验陷阱(写进了附录 B 的注释):libgit2 会缓存第一次用到的配置搜索路径,不在每个用例里显式重设,后面用例的 HOME 切换对它无效,会误以为"libgit2 不读 `init.defaultBranch`"。

**`clone` → 必须留命令行。** 本构建的 libgit2 `https=false ssh=false`:`https://…` 报 `there is no TLS stream available`,`git@…`/`ssh://…` 报 `unsupported URL protocol`。要支持就得给 `git2` 开 `https`/`ssh` feature,拉进 OpenSSL 与 libssh2——与本仓库"走 rustls、不引入 OpenSSL"(见 `dozer-app/Cargo.toml` 里 `reqwest` 的注释)的取向冲突;而且即使开了,libgit2 也读不到 `~/.ssh/config`(主机别名、`ProxyCommand`)、`core.sshCommand`、系统凭据助手的完整配置,用户用命令行能 clone 的仓库用它不一定能。(本机没有可用的网络与凭据环境,**网络 clone 的认证行为我没有实测**,以上依据是上面两条实测的错误信息与 libgit2 的已知限制。)

**`checkout_branch` → 必须留命令行。** 对 10 个场景(干净切换;不冲突的未暂存改动;冲突的未暂存改动;不冲突的已暂存改动;冲突的已暂存改动;未跟踪文件挡路;已暂存的新文件;未暂存的删除;目标就是当前分支;分支不存在),`git2` 的安全 checkout 与命令行**成功/失败一致、最终 HEAD 与工作区一致**。但有三处无法用 `git2` 补上的差异:
1. **不执行 `post-checkout` hook**(命令行执行)。
2. **不执行外部 smudge/clean 过滤器**:配置 `filter.up.smudge = tr a-z A-Z` 后,命令行切换得到 `A2`,`git2` 得到 `a2`。Git LFS 就是靠这个机制,换成 `git2` 会让 LFS 文件以指针文本落在工作区。
3. **报错信息退化**:冲突时命令行给出多行原文(`error: Your local changes to the following files would be overwritten by checkout: a.txt … Please commit your changes or stash them…`,`files/view.rs` 把它原样显示在分支栏下),`git2` 只有 `1 conflict prevents checkout`;分支不存在时的文案也不同。

## 前置条件

- bytegit 仓库 `main` 当前是 `v0.4.0`(P3 的 bytegit 部分已发布)。P5 只新增 `write.rs` 并改 `lib.rs`,与 P3(`stats.rs`)、P4(`watch.rs`)互不依赖。
- **版本号:** 本计划按"当前最新 tag 是 `v0.4.0`"写,发 `v0.5.0`。P4 的计划写的也是"下一档"。**P4、P5 谁后执行,谁以当时最新的 tag 为准再加一档**,并把自己计划里的 `0.5.0`/`v0.5.0` 全部顺延(`grep -n "0\.5\.0" <计划文件>` 能找全);dozer 侧的 `tag` 同理。
- dozer 侧 P1、P2 已合并 `main`;P3 的 dozer 分支(`bytegit-p3`)可能还在进行。P3 与 P5 都会改 `crates/dozer-app/Cargo.toml` 里 `bytegit` 那一行(升 tag),合并顺序决定谁来解这个冲突;P5 分支从 `main` 开,若 P3 先合并了就从合并后的 `main` 开。

## Global Constraints

- bytegit:Rust edition `2024`;公开 API 不得出现 `git2::*`;全部同步;`git2` 保持 `default = []`(**不得**为了 `clone` 开 `https`/`ssh` feature)。
- dozer 侧**所有命令都在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytegit-p5/...`);主 checkout 常有并发会话与未提交改动,不得在其上改代码或 `git add -A`。提交前 `git status` 与 `git diff --cached --stat` 看全貌,只 `git add` 指定路径。
- dozer 的 `bytegit` 依赖一律用 tag;本地联调的 `[patch]` 只放 `.cargo/config.toml`,**不提交**。带着本地 `[patch]` 构建会让 `Cargo.lock` 里 `bytegit`/`byteui` 的 `source = "git+..."` 行消失;提交 `Cargo.lock` 前必须把 `.cargo/config.toml` 里 `bytegit` 的 `[patch]` 段暂时注释掉再 `cargo update -p bytegit`(见 Task 4 Step 1),确认 diff 里 `source` 行都在、只有 bytegit 的 tag 变了。
- 迁移阶段**不得改变用户可见行为**。本计划有意的差异只有"待决事项"里的 D12(只涉及出错时的文案)。
- 新增的 git 操作一律调 `bytegit`(见 `CLAUDE.md`)。`bytegit` 内部为 `clone`/`checkout_branch` 调用 `git` 可执行文件是**评估后的有意选择**,不是遗留。
- dozer 门禁:触碰的文件不产生新诊断、基线之外无新失败(见"已知基线")。`cargo fmt -p dozer-app` 后用 `git status --short` 确认**只有本任务触碰的文件**变了;fmt 动了别的文件就 `git checkout` 回去。
- 变异检验的还原方式:**先 `git add` 暂存当前版本,变异后用 `git checkout -- <文件>` 还原到暂存版本**(新文件不暂存没法还原;未暂存的已修改文件直接 `git checkout` 会把迁移改动一起冲掉)。**不要**做"去掉 `current_dir`"这类会让测试在 bytegit 自己的仓库里跑 `git checkout` 的变异。
- 提交信息用 `feat:`/`fix:`/`refactor:`/`test:`/`docs:` 前缀;提交末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知基线

沿用 P2–P4 计划的"已知基线":`extensions::files::tests::delete_confirm_spec_reflects_pending_target` 在 `main` 上就失败;`cargo build -p dozer-app` 有两条与本计划无关的 dead_code 警告(`usage/aggregate.rs::agent_token_share`、`git_accounts.rs::GitProvider::ALL`);`clippy -D warnings` 有既有错误。另:`extensions::git_log::tests::build_marks_head_branch_and_labels`、`build_populates_time_and_is_merge` 对 dozer 自己的仓库跑 `gleisbau`,**在克隆/linked worktree 里会失败**(旧实现上同样失败,已在 P1 记录;合并回 `main` 后在 `main` 上重跑确认)。**本计划触碰的文件不得新增诊断。**

## Review Focus

规格对下列输入没有逐条说明,但真实使用者会遇到;每条都由指明的任务里的测试固定下来(行为均在草稿里实测确认):

1. **checkout 时工作区有本地改动。** 冲突的改动(未暂存或已暂存)与挡路的未跟踪文件必须让切换失败且工作区原样不动;不冲突的改动(含已暂存的新文件、未暂存的删除)要带到新分支;失败的 `message()` 必须点名受影响的文件(UI 把它原样展示给用户)。bytegit Task 1 的 `a_conflicting_edit_blocks_the_switch_and_leaves_the_worktree_alone`、`an_untracked_file_in_the_way_blocks_the_switch`、`an_unstaged_edit_to_a_file_the_branches_agree_on_is_carried_over`、`a_staged_new_file_is_carried_over`;dozer Task 3 对应的 8 条旧实现测试。
2. **`checkout_branch` 为什么不能换成 `git2`。** `post-checkout` hook 与外部过滤器(Git LFS)必须继续被执行。bytegit Task 1 的 `checkout_branch_runs_the_post_checkout_hook`、`checkout_branch_applies_external_filters`(把评估结论钉成会失败的测试;变异检验 2 会演示换成 `git2` 后它们和三条"报错点名文件"的测试一起失败)。
3. **`clone` 的安全与失败形态。** 用户粘贴或第三方 API 返回的 URL 不可信,必须用 `--` 结束选项解析(CVE-2017-1000117 一类);失败时 `message()` 是 git 的 stderr 原文;机器上没有 `git` 要能区分出来(`GitBinaryUnavailable`)。bytegit Task 1 的 `clone_treats_a_dash_prefixed_url_as_a_path_not_an_option`(断言报错点名那串"选项式 URL",删掉 `--` 它就失败)、`a_missing_git_binary_is_reported_as_binary_unavailable`、`clone_fails_on_a_missing_source_with_gits_stderr`。
4. **`init` 的边界。** 对已有仓库重复 `init` 无害(不改 HEAD 与提交);在已有仓库的子目录里 `init` 是新建嵌套仓库、不是复用上层;目录不存在要报错且**不创建**(libgit2 默认会 `mkdir -p`,必须显式拦住);初始分支遵循用户的 `init.defaultBranch`。bytegit Task 1 的 `init_*` 测试;dozer Task 3 对应的 5 条旧实现测试。
5. **项目目录在仓库子目录里。** `checkout_branch` 迁移前用命令行,在子目录里照样切**整个仓库**(命令行向上查找),所以 dozer 的适配函数必须用 `Repo::discover`(向上查找),**不是** P1/P2 里"只认仓库根"的 `open_exact`——这与那批相反,是有意的。dozer Task 3 的 `checkout_from_a_repo_subdirectory_switches_the_whole_repo`(Task 4 Step 5 的变异检验守着这一点);bytegit Task 1 的 `a_repo_opened_from_a_subdirectory_still_switches_the_whole_repo`。

## 待决事项

- **D11(请确认评估结论):`init` → `git2`;`clone`、`checkout_branch` → 留在 `bytegit` 内部的命令行实现。** 依据见上面"评估结论"。如果将来要让 bytegit 在没有 `git` 可执行文件的环境里工作(比如 Digger 的某种分发形态),代价是:给 `git2` 开 `https`/`ssh` feature(OpenSSL、libssh2)、自己补上 hook 与 LFS 过滤器的执行——我不建议。`bytegit::git_available()` 因此保持**公开**(规格原写"内部检测"):URL 签出表单要在提交前给出"请安装 Xcode Command Line Tools"的提示(`project_create.rs`)。
- **D12(知情):三处出错文案变了(只在出错时展示)。** ① `checkout_branch` 在非仓库目录:命令行的 `fatal: not a git repository…` 变成 libgit2 的 `could not find repository at '<路径>'`;② `init` 在不存在的目录:`无法运行 git: No such file or directory (os error 2)` 变成 `目录不存在: <路径>`;③ `init` 的路径是个文件:同样走 ② 的文案。冲突、分支不存在、`clone` 失败等**主要的**失败文案不变(仍是 git 的 stderr 原文)。
- **D13(知情,保持不修,可另提修复):迁移前就有的三个怪癖原样保留。** ① `git checkout <name>`:分支名与同名文件并存时 git 会报歧义;名字以 `-` 开头会被当成选项——UI 只传 `local_branches()` 里的名字,实际触发不了;修复(`git checkout <name> --` 或拒绝 `-` 开头)会改变部分报错文案,另议。② `clone` 在**从终端启动 dozer** 时,git 可能直接在终端里提示输入凭据,GUI 里那个任务会一直等;从 Finder 启动没有控制终端则直接失败;修复是设 `GIT_TERMINAL_PROMPT=0`,另议。③ `init` 不再生成 14 个 `.sample` 示例 hook(只有一个 `README.sample`),纯外观差异。

规格补充(Task 5 回写):§4.6 的实现选择与评估数据、`git_available` 公开、`CloneOptions` 目前没有可选项(`#[non_exhaustive]`,以后加分支/深度不破坏调用方)、`clone`/`checkout_branch` 的错误分类(`Backend` = git 的 stderr 原文;缺 `git` = `GitBinaryUnavailable`;其他 spawn 失败 = `Io`)、O2 标为已完成。

---

## File Structure

**bytegit 仓库**(`~/Projects/CoralProjects/byteboy/bytegit/`):

| 文件 | 变更 |
|------|------|
| `Cargo.toml` | 版本 `0.4.0` → `0.5.0` |
| `src/write.rs` | 新增:`CloneOptions`、`init`、`clone`、`git_available`、`Repo::checkout_branch`,内部 `run_git`/`spawn_error` |
| `src/lib.rs` | 声明 `mod write;` 并再导出 |

**dozer 仓库**(专用 worktree `~/Projects/CoralProjects/byteboy/dozer-bytegit-p5/`,分支 `bytegit-p5`):

| 文件 | 变更 |
|------|------|
| `crates/dozer-app/src/delivery.rs` | `checkout_branch`/`init_repo`/`git_available`/`clone_repo` 四个函数的函数体换成 bytegit 调用(签名不变);去掉不再使用的 `std::process::Command`(生产代码);追加 13 个刻画测试 |
| `crates/dozer-app/Cargo.toml`、`Cargo.lock` | `bytegit` 的 tag 升到新版本 |
| 规格、`CLAUDE.md`、要求文档、盘点文档 | Task 5 同步 |

调用点(`app/update.rs`、`extensions/files/update.rs`、`extensions/project/scaffold.rs`、`extensions/project_create.rs` 共 6 处)**不动**;适配函数随 P6 一并删除。

---

### Task 1: bytegit — 写操作

**Files:**
- Create: `src/write.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes(已有):`Repo::{discover, root, head, is_empty}`、`GitError`/`GitErrorKind`(含 `GitBinaryUnavailable`)、`TempRepo::{commit_file, branch, checkout, stage, write_untracked, raw_repo, open, path}`
- Produces(Task 2、4 依赖):
  - `fn init(path: &Path) -> Result<Repo, GitError>`(目录必须已存在)
  - `fn clone(url: &str, dest: &Path, opts: CloneOptions) -> Result<Repo, GitError>`
  - `Repo::checkout_branch(&self, name: &str) -> Result<(), GitError>`
  - `fn git_available() -> bool`
  - `struct CloneOptions`(`#[non_exhaustive]`、`Default`、目前无字段)

- [ ] **Step 1: 写 `src/write.rs`(实现 + 测试)**

```rust
//! 写操作:`init`、`clone`、`Repo::checkout_branch`,以及 `git_available`。
//!
//! # 实现选择(评估结论,见规格 §4.6)
//!
//! `init` 用 `git2`。初始分支与命令行一致地遵循 `init.defaultBranch`(libgit2 自己会读全局
//! 配置,没配置为 `master`);与命令行的差异只有示例 hook 文件等无关紧要的内容。
//!
//! `clone`、`checkout_branch` **仍调用 `git` 可执行文件**,调用方看不到命令行细节:
//!
//! - libgit2 在本构建里没有 https/ssh 传输(`git2` 不开 `https`/`ssh` feature,避免引入
//!   OpenSSL/libssh2),网络 URL 直接报 `unsupported URL protocol`;即使开了,也读不到
//!   `~/.ssh/config`、`core.sshCommand`、代理与系统凭据助手的全部配置。
//! - libgit2 的 checkout 在"能不能切、切完什么状态"上与命令行一致,但**不执行
//!   `post-checkout` hook、不执行外部 smudge/clean 过滤器**(Git LFS 会留下指针文件而不是
//!   真实内容),冲突时也只说"N 个冲突"而不列文件、不给处理建议。
//!
//! `checkout_branch_runs_the_post_checkout_hook`、`checkout_branch_applies_external_filters`
//! 两条测试把这个理由钉住:谁想换成 `git2`,先让它们过。

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::Command;

use crate::{GitError, GitErrorKind, Repo};

/// [`clone`] 的选项。目前没有可选项(鉴权完全委托系统已配置的 SSH agent/凭据助手,
/// 不接收 token),留着这个类型是为了以后加分支、深度等选项不破坏调用方。
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CloneOptions {}

/// 机器上有没有可用的 `git` 可执行文件(只看能不能跑起来,不解析版本)。
/// `clone`/`checkout_branch` 依赖它;URL 签出表单在提交前用它给出安装提示。
pub fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// 在已存在的目录 `path` 里新建仓库。仓库已存在时无害(不改 HEAD 与提交)。
/// 初始分支遵循 `init.defaultBranch`(没配置为 `master`)。`path` 不存在时报错且**不创建**
/// 目录(迁移前命令行在不存在的工作目录里根本跑不起来;libgit2 自己会 `mkdir -p`)。
pub fn init(path: &Path) -> Result<Repo, GitError> {
    if !path.is_dir() {
        return Err(GitError::new(
            GitErrorKind::Io,
            format!("目录不存在: {}", path.display()),
        ));
    }
    git2::Repository::init(path)?;
    Repo::discover(path)
}

/// `git clone -- <url> <dest>`。鉴权完全委托系统已配置的 SSH agent/凭据助手。
/// `dest` 应当还不存在(调用方先校验)。失败时 `message()` 是 git 的 stderr 原文;
/// 机器上没有 `git` 返回 `GitErrorKind::GitBinaryUnavailable`。
///
/// `url` 可能来自用户粘贴或第三方 API,两者都不可信:用 `--` 结束选项解析,防止以 `-`
/// 开头的伪造 URL 被 git 当成命令行选项(同 CVE-2017-1000117 那一类问题)。
pub fn clone(url: &str, dest: &Path, _opts: CloneOptions) -> Result<Repo, GitError> {
    run_git(
        "git",
        None,
        &[
            OsStr::new("clone"),
            OsStr::new("--"),
            OsStr::new(url),
            dest.as_os_str(),
        ],
    )?;
    Repo::discover(dest)
}

impl Repo {
    /// 切换到本地分支 `name`(`git checkout <name>`)。工作区有会被覆盖的改动、或有未跟踪
    /// 文件挡路时失败,工作区保持原样;不冲突的改动(含已暂存的)会带到新分支。
    /// 失败时 `message()` 是 git 的 stderr 原文(含受影响的文件与处理建议),可直接展示。
    ///
    /// 从仓库工作区根运行,所以 `Repo` 是从子目录打开的也切整个仓库。
    pub fn checkout_branch(&self, name: &str) -> Result<(), GitError> {
        run_git(
            "git",
            Some(self.root()),
            &[OsStr::new("checkout"), OsStr::new(name)],
        )
    }
}

/// 跑 `program args...`,成功返回 `Ok`;失败带 stderr 原文。`program` 可替换,只为了测试
/// "找不到 git" 的分支。
fn run_git(program: &str, cwd: Option<&Path>, args: &[&OsStr]) -> Result<(), GitError> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().map_err(spawn_error)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(GitError::new(
            GitErrorKind::Backend,
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

fn spawn_error(e: io::Error) -> GitError {
    let kind = if e.kind() == io::ErrorKind::NotFound {
        GitErrorKind::GitBinaryUnavailable
    } else {
        GitErrorKind::Io
    };
    GitError::new(kind, format!("无法运行 git: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempRepo;

    fn read(t: &TempRepo, rel: &str) -> String {
        std::fs::read_to_string(t.path().join(rel)).unwrap()
    }

    /// main:a.txt("a1")、b.txt("b1");feature:a.txt("a2")、多一个 c.txt。当前在 main。
    fn two_branches() -> TempRepo {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a1\n", "base");
        t.commit_file("b.txt", "b1\n", "add b");
        t.branch("feature").checkout("feature");
        t.commit_file("a.txt", "a2\n", "feature changes a");
        t.commit_file("c.txt", "c1\n", "feature adds c");
        t.checkout("main");
        t
    }

    fn head_branch(t: &TempRepo) -> Option<String> {
        t.open().head().unwrap().branch
    }

    // ---- init ----

    #[test]
    fn init_creates_a_repository_with_an_unborn_head() {
        let dir = tempfile::tempdir().unwrap();
        let repo = init(dir.path()).unwrap();
        assert!(dir.path().join(".git").is_dir());
        assert!(repo.is_empty().unwrap());
        assert!(!repo.head().unwrap().has_commits());
    }

    #[test]
    fn init_names_the_initial_branch_after_the_users_default() {
        // 不假设本机配置:预期值用同一份用户配置算出来(没配置为 master)。
        let dir = tempfile::tempdir().unwrap();
        init(dir.path()).unwrap();
        let expected = git2::Config::open_default()
            .and_then(|cfg| cfg.get_string("init.defaultbranch"))
            .ok()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "master".to_string());
        let head = std::fs::read_to_string(dir.path().join(".git/HEAD")).unwrap();
        assert_eq!(head.trim(), format!("ref: refs/heads/{expected}"));
    }

    #[test]
    fn init_on_an_existing_repo_is_harmless() {
        let t = TempRepo::new();
        let id = t.commit_file("a.txt", "x\n", "one");
        let repo = init(t.path()).unwrap();
        let head = repo.head().unwrap();
        assert_eq!(head.branch.as_deref(), Some("main"));
        assert_eq!(head.commit, Some(id));
    }

    #[test]
    fn init_inside_a_repo_subdirectory_creates_a_nested_repo() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "x\n", "one");
        let sub = t.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let nested = init(&sub).unwrap();
        assert!(
            sub.join(".git").is_dir(),
            "在子目录里 init 是新建嵌套仓库,不是复用上层"
        );
        assert_eq!(nested.root(), sub.canonicalize().unwrap());
    }

    #[test]
    fn init_in_a_missing_directory_is_an_error_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        let err = init(&missing).expect_err("目录不存在应报错");
        assert_eq!(err.kind(), GitErrorKind::Io);
        assert!(!missing.exists());
    }

    #[test]
    fn init_on_a_file_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("afile");
        std::fs::write(&file, "x").unwrap();
        assert!(init(&file).is_err());
    }

    // ---- git_available / clone ----

    #[test]
    fn git_available_detects_system_git() {
        // 这几组测试都要求机器上有真 git(`clone`/`checkout_branch` 本来就是命令行实现)。
        assert!(git_available());
    }

    #[test]
    fn clone_copies_a_local_source_repo_and_returns_it() {
        let src = TempRepo::new();
        src.commit_file("a.txt", "one\n", "c1");
        let dest_parent = tempfile::tempdir().unwrap();
        let dest = dest_parent.path().join("cloned");
        let repo = clone(
            &src.path().to_string_lossy(),
            &dest,
            CloneOptions::default(),
        )
        .unwrap();
        assert!(dest.join(".git").exists());
        assert_eq!(
            std::fs::read_to_string(dest.join("a.txt")).unwrap(),
            "one\n"
        );
        assert_eq!(repo.head().unwrap().branch.as_deref(), Some("main"));
    }

    #[test]
    fn clone_fails_on_a_missing_source_with_gits_stderr() {
        let dest_parent = tempfile::tempdir().unwrap();
        let dest = dest_parent.path().join("cloned");
        let missing = dest_parent.path().join("does-not-exist");
        let err = clone(&missing.to_string_lossy(), &dest, CloneOptions::default())
            .expect_err("源不存在应失败");
        assert_eq!(err.kind(), GitErrorKind::Backend);
        assert!(!err.message().is_empty());
    }

    #[test]
    fn clone_treats_a_dash_prefixed_url_as_a_path_not_an_option() {
        // 回归测试:确保 `--` 结束选项解析这道防线还在——若被误删,git 会把这个"URL"解析成
        // `--upload-pack` 选项而不是报"找不到仓库"(同 CVE-2017-1000117 那一类问题)。
        let dest_parent = tempfile::tempdir().unwrap();
        let dest = dest_parent.path().join("cloned");
        let marker = dest_parent.path().join("pwned");
        let url = format!("--upload-pack=touch {}", marker.display());
        let err =
            clone(&url, &dest, CloneOptions::default()).expect_err("伪造的选项应被当成路径而失败");
        assert!(
            !marker.exists(),
            "伪造的 --upload-pack 参数不应该被当成选项执行"
        );
        assert!(!dest.exists());
        // 有 `--` 时 git 把它当成一个(不存在的)仓库路径并在报错里原样点名;
        // 没有 `--` 时它会被当成 `--upload-pack` 选项,报错里只会提到 `dest`。
        assert!(err.message().contains("--upload-pack"), "{}", err.message());
    }

    #[test]
    fn a_missing_git_binary_is_reported_as_binary_unavailable() {
        let err = run_git(
            "definitely-not-a-real-git-binary-for-bytegit-tests",
            None,
            &[OsStr::new("--version")],
        )
        .unwrap_err();
        assert_eq!(err.kind(), GitErrorKind::GitBinaryUnavailable);
        assert!(
            err.message().starts_with("无法运行 git: "),
            "{}",
            err.message()
        );
    }

    // ---- checkout_branch ----

    #[test]
    fn checkout_branch_switches_head_and_the_worktree() {
        let t = two_branches();
        t.open().checkout_branch("feature").unwrap();
        assert_eq!(head_branch(&t).as_deref(), Some("feature"));
        assert_eq!(read(&t, "a.txt"), "a2\n");
        assert_eq!(read(&t, "c.txt"), "c1\n");
    }

    #[test]
    fn checking_out_the_current_branch_is_fine() {
        let t = two_branches();
        t.open().checkout_branch("main").unwrap();
        assert_eq!(head_branch(&t).as_deref(), Some("main"));
    }

    #[test]
    fn an_unstaged_edit_to_a_file_the_branches_agree_on_is_carried_over() {
        let t = two_branches();
        t.write_untracked("b.txt", "b-local\n");
        t.open().checkout_branch("feature").unwrap();
        assert_eq!(read(&t, "b.txt"), "b-local\n");
    }

    #[test]
    fn a_staged_new_file_is_carried_over() {
        let t = two_branches();
        t.write_untracked("d.txt", "d\n").stage("d.txt");
        t.open().checkout_branch("feature").unwrap();
        assert_eq!(read(&t, "d.txt"), "d\n");
    }

    #[test]
    fn a_conflicting_edit_blocks_the_switch_and_leaves_the_worktree_alone() {
        let t = two_branches();
        t.write_untracked("a.txt", "a-local\n");
        let err = t.open().checkout_branch("feature").unwrap_err();
        assert_eq!(err.kind(), GitErrorKind::Backend);
        // git 的 stderr 原文会点名受影响的文件(UI 把它原样展示给用户)。
        assert!(err.message().contains("a.txt"), "{}", err.message());
        assert_eq!(head_branch(&t).as_deref(), Some("main"));
        assert_eq!(read(&t, "a.txt"), "a-local\n");
    }

    #[test]
    fn an_untracked_file_in_the_way_blocks_the_switch() {
        let t = two_branches();
        t.write_untracked("c.txt", "mine\n");
        let err = t.open().checkout_branch("feature").unwrap_err();
        assert!(err.message().contains("c.txt"), "{}", err.message());
        assert_eq!(head_branch(&t).as_deref(), Some("main"));
        assert_eq!(read(&t, "c.txt"), "mine\n");
    }

    #[test]
    fn checking_out_a_missing_branch_is_an_error() {
        let t = two_branches();
        let err = t.open().checkout_branch("nope").unwrap_err();
        assert_eq!(err.kind(), GitErrorKind::Backend);
        assert!(!err.message().is_empty());
        assert_eq!(head_branch(&t).as_deref(), Some("main"));
    }

    #[test]
    fn a_repo_opened_from_a_subdirectory_still_switches_the_whole_repo() {
        let t = two_branches();
        let sub = t.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let repo = Repo::discover(&sub).unwrap();
        repo.checkout_branch("feature").unwrap();
        assert_eq!(head_branch(&t).as_deref(), Some("feature"));
        assert_eq!(read(&t, "a.txt"), "a2\n");
    }

    /// 这条与下一条是"为什么 `checkout_branch` 不能换成 `git2`"的理由,见模块文档。
    #[cfg(unix)]
    #[test]
    fn checkout_branch_runs_the_post_checkout_hook() {
        use std::os::unix::fs::PermissionsExt;
        let t = two_branches();
        let hook = t.path().join(".git/hooks/post-checkout");
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(&hook, "#!/bin/sh\ntouch hook-ran\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        t.open().checkout_branch("feature").unwrap();
        assert!(t.path().join("hook-ran").exists());
    }

    #[cfg(unix)]
    #[test]
    fn checkout_branch_applies_external_filters() {
        // Git LFS 就是靠外部 smudge/clean 过滤器工作的:libgit2 不执行它们,
        // 换成 `git2` 会让 LFS 文件以指针文本落在工作区。
        let t = two_branches();
        std::fs::write(t.path().join(".git/info/attributes"), "a.txt filter=up\n").unwrap();
        let mut cfg = t.raw_repo().config().unwrap();
        cfg.set_str("filter.up.smudge", "tr a-z A-Z").unwrap();
        cfg.set_str("filter.up.clean", "cat").unwrap();
        t.open().checkout_branch("feature").unwrap();
        assert_eq!(read(&t, "a.txt"), "A2\n");
    }
}
```

- [ ] **Step 2: 在 `src/lib.rs` 声明并再导出**

在 `mod status;` 之后加一行 `mod write;`;在文件末尾加 `pub use write::{CloneOptions, clone, git_available, init};`。在 `v0.4.0`(P4 尚未合入)的基础上,`src/lib.rs` 完整为:

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
mod stats;
mod status;
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;
mod write;

pub use change::ChangeKind;
pub use content::{Content, ContentLimits, ContentPair};
pub use error::{GitError, GitErrorKind};
pub use history::{CommitSummary, FileChange, LogOptions, Patch, Signature};
pub use id::{BlobId, CommitId};
pub use info::{HeadInfo, Remote};
pub use repo::Repo;
pub use status::{FileState, StatusEntry, StatusOptions};
pub use write::{CloneOptions, clone, git_available, init};
```

(若 P4 已先合入,`#[cfg(feature = "watch")]` 那几行原样保留,只加上面这两处。)

- [ ] **Step 3: 运行测试与门禁**

Run: `cargo test write:: && cargo test && cargo test --all-features`
Expected: `write::` 21 个通过;全量比基线多 21 个(基线是 P3 发布后的 104 个,即 125 个)。这些测试需要本机有 `git`。

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings`
Expected: 干净。

- [ ] **Step 4: 变异检验(确认 Review Focus 的测试真的会失败)**

先 `git add src/write.rs src/lib.rs` 暂存当前版本;逐个做、逐个用 `git checkout -- src/write.rs` 还原:

1. 删掉 `clone` 里参数列表中的 `OsStr::new("--"),` 那一行。Run: `cargo test write::`。Expected: `clone_treats_a_dash_prefixed_url_as_a_path_not_an_option` 失败。
2. 把 `Repo::checkout_branch` 的函数体换成基于 `git2` 的实现(演示"为什么不能换"):

   ```rust
   let repo = self.raw();
   let refname = format!("refs/heads/{name}");
   let obj = repo.revparse_single(&refname)?;
   let mut cb = git2::build::CheckoutBuilder::new();
   cb.safe();
   repo.checkout_tree(&obj, Some(&mut cb))?;
   repo.set_head(&refname)?;
   Ok(())
   ```

   Expected: 5 个测试失败——`checkout_branch_runs_the_post_checkout_hook`、`checkout_branch_applies_external_filters`、`a_conflicting_edit_blocks_the_switch_and_leaves_the_worktree_alone`、`an_untracked_file_in_the_way_blocks_the_switch`(报错不再点名文件)、`checking_out_a_missing_branch_is_an_error`(libgit2 把"找不到分支"归为别的错误分类,不再是 `Backend`)。
3. 删掉 `init` 里 `if !path.is_dir() { ... }` 那一段(五行)。Expected: `init_in_a_missing_directory_is_an_error_and_creates_nothing` 失败。
4. 把 `spawn_error` 里的 `if e.kind() == io::ErrorKind::NotFound {` 改成 `if false {`。Expected: `a_missing_git_binary_is_reported_as_binary_unavailable` 失败。

- [ ] **Step 5: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git status --short
git add src/write.rs src/lib.rs
git diff --cached --stat
git commit -m "feat: add init, clone, checkout_branch and git_available

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: bytegit — 发布 v0.5.0

**Files:**
- Modify: `Cargo.toml`(`version = "0.5.0"`)

**Interfaces:** Produces:git tag `v0.5.0`(Task 4 的 dozer 依赖)。

- [ ] **Step 1: 升版本并全量门禁**

把 `Cargo.toml` 的 `version = "0.4.0"` 改为 `"0.5.0"`(版本号顺延规则见"前置条件")。

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings && cargo test && cargo test --all-features`
Expected: 全部通过(基线 + 21 个)。

- [ ] **Step 2: Commit 并推送 main(对外可见操作:先向用户确认)**

向用户确认"现在推送 `main` 并打 tag `v0.5.0`"。获得明确同意后:

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git add Cargo.toml
git commit -m "chore: release 0.5.0

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push origin main
gh run watch "$(gh run list --repo byteboyai/bytegit --limit 1 --json databaseId -q '.[0].databaseId')" --repo byteboyai/bytegit --exit-status
```

Expected: CI(macOS,runner 上有 `git`)通过。**CI 红则先修,不打 tag。**

- [ ] **Step 3: 打 tag 并验证可被消费**

```bash
git tag -a v0.5.0 -m "bytegit v0.5.0: init, clone, checkout_branch, git_available"
git push origin v0.5.0
cd "$(mktemp -d)" && cargo new --lib consume -q && cd consume
cargo add bytegit --git https://github.com/byteboyai/bytegit --tag v0.5.0 --features testutil
cat >> src/lib.rs <<'EOF'

#[cfg(test)]
mod t {
    #[test]
    fn consumes_bytegit_write_ops() {
        assert!(bytegit::git_available());
        let src = bytegit::testutil::TempRepo::new();
        src.commit_file("a.txt", "x\n", "one");
        src.branch("feature");
        let parent = tempfile::tempdir().unwrap();
        let dest = parent.path().join("cloned");
        let repo = bytegit::clone(
            &src.path().to_string_lossy(),
            &dest,
            bytegit::CloneOptions::default(),
        )
        .unwrap();
        assert_eq!(repo.head().unwrap().branch.as_deref(), Some("main"));

        let fresh = tempfile::tempdir().unwrap();
        assert!(bytegit::init(fresh.path()).unwrap().is_empty().unwrap());
    }
}
EOF
cargo add --dev tempfile -q
cargo test
```

Expected: `consumes_bytegit_write_ops ... ok`。

---

### Task 3: dozer — 在旧实现上补刻画测试

**Files:**
- Modify(worktree 内,只改 `tests` 模块): `crates/dozer-app/src/delivery.rs`

**Interfaces:** 无新接口。产出 13 个刻画测试(`init_repo` 5 个、`checkout_branch` 8 个),在**旧实现**上全部通过;Task 4 换函数体后它们(以及原有的 `clone_repo_*`、`git_available_*` 测试)保持不变、继续通过。

- [ ] **Step 1: 建专用 worktree 与分支**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytegit-p5 -b bytegit-p5 main
mkdir -p ../dozer-bytegit-p5/.cargo && cp .cargo/config.toml ../dozer-bytegit-p5/.cargo/config.toml
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p5 && git log --oneline | head -1 && git status --short
```

Expected: 干净、分支 `bytegit-p5`,HEAD 与 `main` 一致。`.cargo/config.toml`(不提交,已被 `.gitignore`)里的 `[patch."https://github.com/byteboyai/bytegit"]` 指向本地 `../bytegit`,此时它已含 Task 1–2 的改动。此后**所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytegit-p5/` 前缀**。

- [ ] **Step 2: 给 `delivery.rs` 的 `tests` 模块加 13 个刻画测试**

在 `crates/dozer-app/src/delivery.rs` 的 `mod tests` 末尾(最后一个测试之后、模块结尾 `}` 之前)追加(用的是该模块已有的 `mkrepo()`、`git_in()`、`branch()`、`repo_root()`、`current_branch_has_commits()`):

```rust
    // ---- bytegit P5:init_repo / checkout_branch 迁移前后必须一致的口径 ----

    /// main:a.txt("a1")、b.txt("b1");feature:a.txt("a2")、多一个 c.txt。当前在 main。
    fn two_branches() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        git_in(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a1\n").unwrap();
        std::fs::write(repo.join("b.txt"), "b1\n").unwrap();
        git_in(&repo, &["add", "."]);
        git_in(&repo, &["commit", "-qm", "base"]);
        git_in(&repo, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(repo.join("a.txt"), "a2\n").unwrap();
        std::fs::write(repo.join("c.txt"), "c1\n").unwrap();
        git_in(&repo, &["add", "."]);
        git_in(&repo, &["commit", "-qm", "feature"]);
        git_in(&repo, &["checkout", "-q", "main"]);
        (dir, repo)
    }

    fn read(repo: &std::path::Path, rel: &str) -> String {
        std::fs::read_to_string(repo.join(rel)).unwrap()
    }

    #[test]
    fn init_repo_creates_a_repository_with_no_commits() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path()).unwrap();
        assert!(dir.path().join(".git").is_dir());
        assert!(repo_root(dir.path()).is_some());
        assert!(!current_branch_has_commits(dir.path()));
        assert_eq!(branch(dir.path()), None);
    }

    #[test]
    fn init_repo_uses_the_users_default_branch_name() {
        // 不假设本机配置:预期值用 `git config` 读同一份。
        let out = Command::new("git")
            .args(["config", "--get", "init.defaultBranch"])
            .output()
            .unwrap();
        let configured = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let expected = if configured.is_empty() {
            "master".to_string()
        } else {
            configured
        };
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path()).unwrap();
        let head = std::fs::read_to_string(dir.path().join(".git/HEAD")).unwrap();
        assert_eq!(head.trim(), format!("ref: refs/heads/{expected}"));
    }

    #[test]
    fn init_repo_on_an_existing_repo_keeps_its_commits() {
        let (_d, repo) = mkrepo();
        let before = branch(&repo);
        init_repo(&repo).unwrap();
        assert_eq!(branch(&repo), before);
        assert!(current_branch_has_commits(&repo));
    }

    #[test]
    fn init_repo_inside_a_repo_subdirectory_creates_a_nested_repo() {
        let (_d, repo) = mkrepo();
        let sub = repo.join("sub");
        std::fs::create_dir(&sub).unwrap();
        init_repo(&sub).unwrap();
        assert!(sub.join(".git").is_dir());
    }

    #[test]
    fn init_repo_fails_when_the_directory_is_missing_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert!(init_repo(&missing).is_err());
        assert!(!missing.exists());
    }

    #[test]
    fn checkout_branch_switches_head_and_the_worktree() {
        let (_d, repo) = two_branches();
        checkout_branch(&repo, "feature").unwrap();
        assert_eq!(branch(&repo).as_deref(), Some("feature"));
        assert_eq!(read(&repo, "a.txt"), "a2\n");
        assert_eq!(read(&repo, "c.txt"), "c1\n");
    }

    #[test]
    fn checking_out_the_current_branch_is_fine() {
        let (_d, repo) = two_branches();
        checkout_branch(&repo, "main").unwrap();
        assert_eq!(branch(&repo).as_deref(), Some("main"));
    }

    #[test]
    fn checkout_carries_an_unstaged_edit_to_a_file_the_branches_agree_on() {
        let (_d, repo) = two_branches();
        std::fs::write(repo.join("b.txt"), "b-local\n").unwrap();
        checkout_branch(&repo, "feature").unwrap();
        assert_eq!(read(&repo, "b.txt"), "b-local\n");
    }

    #[test]
    fn checkout_carries_a_staged_new_file() {
        let (_d, repo) = two_branches();
        std::fs::write(repo.join("d.txt"), "d\n").unwrap();
        git_in(&repo, &["add", "d.txt"]);
        checkout_branch(&repo, "feature").unwrap();
        assert_eq!(read(&repo, "d.txt"), "d\n");
    }

    #[test]
    fn a_conflicting_edit_blocks_checkout_and_the_error_names_the_file() {
        let (_d, repo) = two_branches();
        std::fs::write(repo.join("a.txt"), "a-local\n").unwrap();
        let err = checkout_branch(&repo, "feature").unwrap_err();
        // git 的 stderr 原文会点名受影响的文件(`git_error` 把它原样展示给用户)。
        assert!(err.contains("a.txt"), "{err}");
        assert_eq!(branch(&repo).as_deref(), Some("main"));
        assert_eq!(read(&repo, "a.txt"), "a-local\n");
    }

    #[test]
    fn an_untracked_file_in_the_way_blocks_checkout() {
        let (_d, repo) = two_branches();
        std::fs::write(repo.join("c.txt"), "mine\n").unwrap();
        let err = checkout_branch(&repo, "feature").unwrap_err();
        assert!(err.contains("c.txt"), "{err}");
        assert_eq!(branch(&repo).as_deref(), Some("main"));
        assert_eq!(read(&repo, "c.txt"), "mine\n");
    }

    #[test]
    fn checking_out_a_missing_branch_is_an_error() {
        let (_d, repo) = two_branches();
        let err = checkout_branch(&repo, "nope").unwrap_err();
        assert!(!err.is_empty());
        assert_eq!(branch(&repo).as_deref(), Some("main"));
    }

    #[test]
    fn checkout_from_a_repo_subdirectory_switches_the_whole_repo() {
        let (_d, repo) = two_branches();
        let sub = repo.join("sub");
        std::fs::create_dir(&sub).unwrap();
        checkout_branch(&sub, "feature").unwrap();
        assert_eq!(branch(&repo).as_deref(), Some("feature"));
        assert_eq!(read(&repo, "a.txt"), "a2\n");
    }
```

- [ ] **Step 3: 在旧实现上运行,确认全部通过**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p5 && cargo fmt -p dozer-app && git status --short`
Expected: 只有 `crates/dozer-app/src/delivery.rs` 变了。

Run: `cargo test -p dozer-app delivery::`
Expected: 42 个全部通过(原有 29 个 + 新增 13 个)。**任何新增测试在旧实现上失败,说明我对旧行为的描述有误——先停下确认旧行为,不要改实现,也不要直接改断言迎合。**(这些测试依赖本机 `git` 与 `git init -b`,需要 git ≥ 2.28。)

- [ ] **Step 4: 门禁与 Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p5
git add crates/dozer-app/src/delivery.rs
git diff --cached --stat
git commit -m "test(delivery): characterize init_repo and checkout_branch before bytegit P5 migration

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: dozer — `delivery` 的四个函数换成 bytegit 调用

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`、`Cargo.lock`、`crates/dozer-app/src/delivery.rs`

**Interfaces:**
- Consumes: bytegit 新版本的 `init`、`clone`、`CloneOptions`、`git_available`、`Repo::{discover, checkout_branch}`
- Produces: `delivery::{checkout_branch(&Path, &str) -> Result<(), String>, init_repo(&Path) -> Result<(), String>, git_available() -> bool, clone_repo(&str, &Path) -> Result<(), String>}`——**签名不变**,6 个调用点不动

- [ ] **Step 1: 升依赖、让 `Cargo.lock` 指向 tag**

1. `crates/dozer-app/Cargo.toml`:`bytegit = { ... tag = "..." }` 的 tag 改成 Task 2 发布的版本(通常是 `v0.5.0`)。
2. 让 `Cargo.lock` 指向真实 tag(而不是本地 patch):临时把 `.cargo/config.toml` 里 `[patch."https://github.com/byteboyai/bytegit"]` 与下一行注释掉(保留 byteui 的 patch),运行 `cargo update -p bytegit`,再 `git diff Cargo.lock` 确认**只有** `bytegit` 的 version 与 `source` 的 tag 变化,`source = "git+..."` 行都在。确认后恢复 `.cargo/config.toml`(之后若 `cargo build` 又去掉了 `source` 行,提交前 `git checkout Cargo.lock` 还原再重复本步)。

- [ ] **Step 2: 换函数体**

在 `crates/dozer-app/src/delivery.rs`:

1. 把原来从 `/// 切换到 `name` 指定分支(本地分支)。` 起、到 `#[cfg(test)]\nmod tests {` 之前的四个函数(`checkout_branch`、`init_repo`、`git_available`、`clone_repo`,含各自的文档注释)整体替换为:

```rust
/// 切换到 `name` 指定分支(本地分支)。错误透传 git 的 stderr,便于展示给
/// 用户(如工作区有未提交改动导致的 checkout 失败)。实现是 `bytegit` 的
/// `Repo::checkout_branch`(仍走 `git` 命令行:hook 与 Git LFS 过滤器要执行,
/// 冲突时要给出带文件列表的原文——见 `bytegit` 的 `write.rs` 模块文档)。
/// 项目目录在仓库子目录里时向上查找,切整个仓库(与迁移前命令行一致)。
pub fn checkout_branch(repo: &Path, name: &str) -> Result<(), String> {
    Repo::discover(repo)
        .and_then(|r| r.checkout_branch(name))
        .map_err(|e| e.message().to_string())
}

/// 在项目根目录新建 git 仓库。初始分支遵循用户的 `init.defaultBranch`。
/// 错误以可展示的文本返回。
pub fn init_repo(repo: &Path) -> Result<(), String> {
    bytegit::init(repo)
        .map(|_| ())
        .map_err(|e| e.message().to_string())
}

/// 系统是否装了可用的 git——URL 签出 tab 提交前的轻量检测,不解析
/// 具体版本号,只看子进程能否成功跑起来。
pub fn git_available() -> bool {
    bytegit::git_available()
}

/// `git clone <url> <dest>`,鉴权完全委托系统已配置的 SSH agent/凭证
/// 管理器(不接 `RemoteCallbacks`,不做任何 Token 输入,见
/// `docs/superpowers/specs/2026-09-18-new-project-creation-design.md`)。
/// `dest` 必须还不存在(调用方在此之前已经校验过,见 `project_create`
/// 模块的 `validate_target_not_exists`),失败把 git 的 stderr 原样透传。
/// `url` 可能来自用户直接粘贴,也可能来自第三方 API 返回的 clone_url
/// (见 `git_accounts::list_repos`)——两者都不可信;`--` 结束选项解析的
/// 防线在 `bytegit::clone` 里(同 CVE-2017-1000117 那一类问题)。
pub fn clone_repo(url: &str, dest: &Path) -> Result<(), String> {
    bytegit::clone(url, dest, bytegit::CloneOptions::default())
        .map(|_| ())
        .map_err(|e| e.message().to_string())
}
```

2. `use` 区:删掉 `use std::process::Command;`(生产代码不再使用;`mod tests` 里自己有一份 `use std::process::Command;`,不动)。

- [ ] **Step 3: 编译并运行**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p5 && cargo fmt -p dozer-app && git status --short && cargo build -p dozer-app 2>&1 | grep -E "^(warning|error)" -A6`
Expected: `git status` 只有本任务列出的文件;无 error;警告只有"已知基线"里与本计划无关的两条。

Run: `cargo test -p dozer-app delivery::`
Expected: 42 个全部通过(Task 3 的 13 个与原有的 `clone_repo_*`、`git_available_*` 一字未改)。

Run: `cargo test -p dozer-app project_create && cargo test -p dozer-app scaffold && cargo test -p dozer-app 2>&1 | grep -E "FAILED|test result"`
Expected: 通过数与 Task 3 之后(迁移前)一致;失败只有"已知基线"里列的(`delete_confirm_spec_reflects_pending_target`,以及在克隆/worktree 里那两条 `git_log` 测试)。

- [ ] **Step 4: 门禁**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "delivery.rs"`
Expected: 无输出。

Run: `grep -n "Command::new" crates/dozer-app/src/delivery.rs | head -3; grep -n "cfg(test)" crates/dozer-app/src/delivery.rs`
Expected: `Command::new` 的行号都大于 `#[cfg(test)]` 的行号(只剩测试夹具)。

- [ ] **Step 5: 变异检验**

先 `git add crates/dozer-app/src/delivery.rs` 暂存当前版本。把 `checkout_branch` 里的 `Repo::discover(repo)` 换成 `open_exact(repo).ok_or_else(|| "not a repo root".to_string())`(并相应调整后面的链式调用,使其仍能编译,例如 `.and_then(|r| r.checkout_branch(name).map_err(|e| e.message().to_string()))`)。Run: `cargo test -p dozer-app delivery::`。Expected: `checkout_from_a_repo_subdirectory_switches_the_whole_repo` 失败。用 `git checkout -- crates/dozer-app/src/delivery.rs` 还原。

- [ ] **Step 6: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p5
git status --short
git add Cargo.lock crates/dozer-app/Cargo.toml crates/dozer-app/src/delivery.rs
git diff --cached --stat
git commit -m "refactor(delivery): back init/clone/checkout/git_available with bytegit

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: `git diff --cached --stat` 里没有 `.cargo/config.toml`。

---

### Task 5: 文档同步与收尾

**Files:**
- Modify(worktree 内): `CLAUDE.md`、`docs/superpowers/specs/2026-10-02-bytegit-design.md`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`、`docs/dozer-v2/bytegit-调用点盘点.md`
- Create: `docs/superpowers/specs/2026-10-03-bytegit-write-ops-evaluation.md`(把附录 A、B 与评估数据存档,以后有人想换 `git2` 时先看)
- Modify(记忆): `~/.claude/projects/-Users-chrischiang-Projects-CoralProjects-byteboy-dozer/memory/dozer-v2-panel-independence-and-bytegit.md`

**Interfaces:** 无代码接口。

- [ ] **Step 1: `CLAUDE.md` 的 bytegit 条目更新进度**

把"已迁移/未迁移"的描述更新为:P5(`init/clone/checkout_branch/git_available`)已迁移到 `bytegit`;**`bytegit` 内部为 `clone`/`checkout_branch` 仍调用 `git` 可执行文件是评估后的有意选择**(libgit2 在本构建没有 https/ssh 传输、不执行 `post-checkout` hook 与外部过滤器/Git LFS、冲突报错不点名文件),不要"顺手"换成 `git2`——换之前先让 `bytegit/src/write.rs` 里的钉死测试过。`delivery.rs` 剩下的是 bytegit 适配层,P6 移除。若条目里还没有 P1–P4 的措辞(对应分支尚未合并),只改与 P5 相关的部分,不要替它们写。

- [ ] **Step 2: 写评估存档**

新建 `docs/superpowers/specs/2026-10-03-bytegit-write-ops-evaluation.md`,内容:

1. 环境(macOS、git 版本、libgit2 版本、`git2` feature 状态);
2. 本计划"评估结论"一节的三段(`init`/`clone`/`checkout_branch`),原样搬过去,包括 10 个 checkout 场景的清单与三处无法补上的差异;
3. 附录 A、B 的两个程序(完整源码)与它们的原始输出;
4. 一句话说明如何重跑:在临时 crate 里 `cargo add git2@0.21 tempfile`,把两个文件放进 `src/bin/`,`cargo run --bin init_clone`、`cargo run --bin checkout`。

- [ ] **Step 3: 规格同步**

在 `docs/superpowers/specs/2026-10-02-bytegit-design.md`:

1. §4.6 与实现和评估结论对齐:
   - `init(path)` 用 `git2`,目录必须已存在(不创建);初始分支遵循 `init.defaultBranch`;
   - `clone(url, dest, CloneOptions)`、`Repo::checkout_branch(name)` **在 bytegit 内部调用 `git` 可执行文件**,失败时 `message()` 是 git 的 stderr 原文(`GitErrorKind::Backend`),缺 `git` 为 `GitBinaryUnavailable`,其他 spawn 失败为 `Io`;`clone` 用 `--` 结束选项解析;
   - `git_available()` **公开**(URL 签出表单需要在提交前提示安装);
   - `CloneOptions` 目前没有可选项,`#[non_exhaustive]`;
   - 把"并行评估项"一段改为"评估结论"并指向存档文档,三条结论(`init`→`git2`、`clone`/`checkout`→命令行)与依据(本构建 libgit2 无 https/ssh;不执行 hook 与外部过滤器;报错信息退化)。
2. §6 P5 一行:验收补充"刻画测试在旧/新实现上都通过";"删除的旧代码"注明是 `delivery.rs` 里四个函数的命令行实现(适配函数保留到 P6)。
3. §8:O2 标为已完成并写明结论;加 **O16**(= 本计划 D13 的三个怪癖:`checkout` 的歧义/选项式名字、`clone` 的终端凭据提示、`init` 的示例 hook),均为迁移前就有、保持原样。

- [ ] **Step 4: 要求文档、盘点文档、记忆**

- `docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` §7 第 1 条后追加:`bytegit` P5 已完成(写操作评估:`init` 用 `git2`,`clone`/`checkout` 留命令行,理由见评估存档)。
- `docs/dozer-v2/bytegit-调用点盘点.md`:在 §1 末尾加一行"P5 之后,生产代码里直接调用命令行 `git` 的只剩 `bytegit` 内部的 `clone`/`checkout_branch`";并 `grep -n "init_repo\|clone_repo\|checkout_branch\|git_available\|Command::new" docs/dozer-v2/bytegit-调用点盘点.md`,把命中的调用点标注为"P5 已迁移"。
- 记忆文件追加 P5 结果、评估结论与待决事项 D11/D12/D13(一两行),并更新分支(`bytegit-p5`)的合并现状。

- [ ] **Step 5: Commit(worktree)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p5
git status --short
git add CLAUDE.md docs/superpowers/specs/2026-10-02-bytegit-design.md docs/superpowers/specs/2026-10-03-bytegit-write-ops-evaluation.md docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md docs/dozer-v2/bytegit-调用点盘点.md
git diff --cached --stat
git commit -m "docs(bytegit): record P5 results and the write-ops evaluation

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: 交给用户合并**

不要自己合并 `bytegit-p5` 到 `main`:向用户报告分支、提交列表、"已知基线"里的失败、待决事项 D11/D12/D13(**尤其请用户确认 D11 的评估结论**),并请用户审阅后决定合并方式。合并前请用户在真实项目上手点一遍:① 右键文件树/分支栏切分支(有无冲突改动各一次,看分支栏下的报错是不是 git 的原文);② 空目录上点"初始化 git 仓库";③ 新建项目 → 从 URL 克隆(一个 https、一个 ssh,各一次);④ 一个带 `post-checkout` hook 或 LFS 的仓库切分支。**这些交互没有自动化覆盖,只靠编译与上面的测试。**

---

## Self-Review

**Spec coverage(规格 §6 的 P5 行与 §4.6、O2):**
- `init`/`clone`/`Repo::checkout_branch`:Task 1;`git_available` 保留(改为公开,D11 说明理由);`GitBinaryUnavailable`:Task 1 的 `run_git`/`spawn_error` 与测试;发布:Task 2。
- "并行评估项"的评估结论回写规格:本计划"评估结论"一节有数据与依据,Task 5 Step 2、3 存档并回写;O2 标为完成。
- 删除 `delivery.rs` 剩余的命令行函数:Task 4 把四个函数的函数体换成 bytegit 调用、去掉生产代码里的 `Command`;适配函数本身保留到 P6(与 P1 的做法一致,调用点 6 处不动),这一点与规格 P5 行"删除的旧代码:`delivery.rs` 剩余的命令行函数"一致(命令行实现已删,只剩适配层)。
- 规格没提到的 `CloneOptions` 字段:无可选项,`non_exhaustive`,Task 5 回写。

**Placeholder scan:** 无 TBD/TODO;Task 1 给了完整文件,Task 3 给了完整测试,Task 4 给了完整的替换代码。

**Type consistency:** `init`/`clone`/`CloneOptions`/`git_available`/`Repo::checkout_branch` 在 Task 1 定义,Task 4 的适配函数使用同名同形;`delivery` 四个函数的签名与迁移前相同,6 个调用点不改。

**Review Focus:** 五条各自有对应测试(分别见各条所指任务),其中 bytegit 侧 4 个变异检验、dozer 侧 1 个变异检验已在草稿里实际确认会让指定测试失败;交互层面(UI 报错展示、真实网络 clone)没有自动化覆盖,Task 5 Step 6 列了人工验收清单。

---

## 附录 A:评估程序——`init` 与 `clone`(`src/bin/init_clone.rs`)

```rust
//! 评估 `init`/`clone` 能否用 git2 替代命令行(bytegit P5)。
//! 结论:`init` 可以(libgit2 自己会读 `init.defaultBranch`,下面显式接线与否结果相同);
//! `clone` 不行(本构建的 libgit2 没有 https/ssh 传输)。
//! 注意:libgit2 会缓存第一次用到的配置搜索路径,每个用例必须显式重设,否则后面用例的
//! HOME 切换对它无效、会得出错误结论。
//! 运行:`cargo run --bin init_clone`(需要系统 git;只依赖 git2 与 tempfile,不开 https/ssh feature)。
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

fn cli_init(dir: &Path, home: &Path) -> bool {
    Command::new("git")
        .arg("init")
        .current_dir(dir)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("xdg"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .output()
        .unwrap()
        .status
        .success()
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
    let mut es: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    es.sort_by_key(|e| e.file_name());
    for e in es {
        let p = e.path();
        let rel = p.strip_prefix(base).unwrap().to_string_lossy().to_string();
        if rel.starts_with("objects/") && rel != "objects/info" && rel != "objects/pack" {
            continue;
        }
        out.push(rel);
        if p.is_dir() {
            walk(base, &p, out);
        }
    }
}

/// (.git 下的文件清单, HEAD 内容, config 键值)
fn snapshot(repo: &Path) -> (Vec<String>, String, BTreeMap<String, String>) {
    let g = repo.join(".git");
    let mut files = vec![];
    walk(&g, &g, &mut files);
    let head = std::fs::read_to_string(g.join("HEAD")).unwrap().trim().to_string();
    let mut cfg = BTreeMap::new();
    for l in std::fs::read_to_string(g.join("config")).unwrap().lines() {
        if let Some((k, v)) = l.trim().split_once('=') {
            cfg.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    (files, head, cfg)
}

/// `with_default_branch`:git2 一侧是否显式读 `init.defaultBranch` 再传给 `initial_head`。
fn init_case(label: &str, gitconfig: &str, template: bool, with_default_branch: bool) {
    let home = tempfile::tempdir().unwrap();
    let tpl = home.path().join("tpl");
    std::fs::write(
        home.path().join(".gitconfig"),
        gitconfig.replace("TPL", &tpl.to_string_lossy()),
    )
    .unwrap();
    if template {
        std::fs::create_dir_all(tpl.join("hooks")).unwrap();
        std::fs::write(tpl.join("hooks").join("post-checkout"), "#!/bin/sh\n").unwrap();
    }
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let cli_ok = cli_init(a.path(), home.path());
    // libgit2 缓存第一次用到的配置搜索路径,每个用例都要显式重设。
    unsafe {
        git2::opts::set_search_path(git2::ConfigLevel::Global, home.path()).unwrap();
        git2::opts::set_search_path(git2::ConfigLevel::XDG, home.path().join("xdg")).unwrap();
        git2::opts::set_search_path(git2::ConfigLevel::System, home.path().join("nosystem")).unwrap();
    }
    let mut opts = git2::RepositoryInitOptions::new();
    if with_default_branch {
        if let Ok(name) = git2::Config::open_default().and_then(|c| c.get_string("init.defaultbranch")) {
            opts.initial_head(&name);
        }
    }
    let g2_ok = git2::Repository::init_opts(b.path(), &opts).is_ok();
    let ((fa, ha, ca), (fb, hb, cb)) = (snapshot(a.path()), snapshot(b.path()));
    println!("== init [{label}] default-branch-wired={with_default_branch} cli_ok={cli_ok} git2_ok={g2_ok}");
    println!("   HEAD cli={ha:?} git2={hb:?}");
    println!("   files only in cli : {} 个(如 {:?})", fa.iter().filter(|x| !fb.contains(x)).count(), fa.iter().find(|x| !fb.contains(x)));
    println!("   files only in git2: {:?}", fb.iter().filter(|x| !fa.contains(x)).collect::<Vec<_>>());
    let keys: BTreeSet<_> = ca.keys().chain(cb.keys()).cloned().collect();
    let diffs: Vec<_> = keys.into_iter().filter(|k| ca.get(k) != cb.get(k)).collect();
    println!("   config 键差异: {diffs:?}");
}

fn main() {
    init_case("no config", "", false, false);
    init_case("init.defaultBranch=trunk", "[init]\n\tdefaultBranch = trunk\n", false, false);
    init_case("init.defaultBranch=trunk", "[init]\n\tdefaultBranch = trunk\n", false, true);
    init_case("init.templateDir custom", "[init]\n\ttemplateDir = TPL\n", true, true);

    let d = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    cli_init(d.path(), home.path());
    println!("== reinit existing repo: git2_ok={}", git2::Repository::init(d.path()).is_ok());
    let missing = d.path().join("nope");
    println!(
        "== init in nonexistent dir: git2_ok={} (created the dir: {})",
        git2::Repository::init(&missing).is_ok(),
        missing.exists()
    );
    let file = d.path().join("afile");
    std::fs::write(&file, "x").unwrap();
    println!("== init on a file path: git2_ok={}", git2::Repository::init(&file).is_ok());

    let t = tempfile::tempdir().unwrap();
    for url in ["https://example.invalid/x.git", "git@example.invalid:x/y.git", "ssh://git@example.invalid/x/y.git"] {
        let r = git2::Repository::clone(url, t.path().join("c"));
        println!("== git2 clone {url}: ok={} err={:?}", r.is_ok(), r.err().map(|e| e.message().to_string()));
        let _ = std::fs::remove_dir_all(t.path().join("c"));
    }
    let v = git2::Version::get();
    println!("libgit2 {:?}: https={} ssh={}", v.libgit2_version(), v.https(), v.ssh());
}
```

## 附录 B:评估程序——`checkout`(`src/bin/checkout.rs`)

```rust
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new("git").args(args).current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME","t").env("GIT_AUTHOR_EMAIL","t@t").env("GIT_COMMITTER_NAME","t").env("GIT_COMMITTER_EMAIL","t@t")
        .output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).trim().to_string())
}

/// main: a.txt b.txt ; feature: a.txt changed, c.txt added. 当前在 main。
fn fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap(); let p = d.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "a1\n").unwrap(); std::fs::write(p.join("b.txt"), "b1\n").unwrap();
    git(p, &["add", "."]); git(p, &["commit", "-qm", "base"]);
    git(p, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(p.join("a.txt"), "a2\n").unwrap(); std::fs::write(p.join("c.txt"), "c1\n").unwrap();
    git(p, &["add", "."]); git(p, &["commit", "-qm", "feature"]);
    git(p, &["checkout", "-q", "main"]);
    d
}

fn git2_checkout(repo: &Path, name: &str) -> Result<(), String> {
    let r = git2::Repository::open(repo).map_err(|e| e.message().to_string())?;
    let (obj, reference) = r.revparse_ext(&format!("refs/heads/{name}")).map_err(|e| e.message().to_string())?;
    let _ = reference;
    let mut cb = git2::build::CheckoutBuilder::new();
    cb.safe();
    r.checkout_tree(&obj, Some(&mut cb)).map_err(|e| e.message().to_string())?;
    r.set_head(&format!("refs/heads/{name}")).map_err(|e| e.message().to_string())?;
    Ok(())
}

fn state(p: &Path) -> String {
    let (_, head) = git(p, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let (_, st) = git(p, &["status", "--porcelain"]);
    let mut files = vec![];
    for e in std::fs::read_dir(p).unwrap().flatten() { let n = e.file_name().to_string_lossy().to_string(); if n != ".git" { files.push(format!("{n}={:?}", std::fs::read_to_string(e.path()).unwrap_or_default().trim())); } }
    files.sort();
    format!("HEAD={head} | status={:?} | files={files:?}", st.replace('\n', ";"))
}

fn scenario(label: &str, target: &str, setup: &dyn Fn(&Path)) {
    let a = fixture(); let b = fixture();
    setup(a.path()); setup(b.path());
    let (ok, msg) = git(a.path(), &["checkout", target]);
    let r = git2_checkout(b.path(), target);
    let sa = state(a.path()); let sb = state(b.path());
    let same = ok == r.is_ok() && sa == sb;
    println!("-- {label}: cli_ok={ok} git2_ok={} {}", r.is_ok(), if same { "SAME-STATE" } else { "DIFFERENT" });
    if !same { println!("   cli  : {sa}\n   git2 : {sb}"); }
    println!("   cli msg : {:?}", msg.replace('\n', " / ").chars().take(150).collect::<String>());
    println!("   git2 msg: {:?}", r.err().unwrap_or_default());
}

fn main() {
    scenario("clean switch", "feature", &|_| {});
    scenario("unstaged edit in b.txt (same on both branches)", "feature", &|p| std::fs::write(p.join("b.txt"), "b-local\n").unwrap());
    scenario("unstaged edit in a.txt (differs between branches)", "feature", &|p| std::fs::write(p.join("a.txt"), "a-local\n").unwrap());
    scenario("staged edit in b.txt", "feature", &|p| { std::fs::write(p.join("b.txt"), "b-staged\n").unwrap(); git(p, &["add", "b.txt"]); });
    scenario("staged edit in a.txt (differs)", "feature", &|p| { std::fs::write(p.join("a.txt"), "a-staged\n").unwrap(); git(p, &["add", "a.txt"]); });
    scenario("untracked c.txt would be overwritten", "feature", &|p| std::fs::write(p.join("c.txt"), "mine\n").unwrap());
    scenario("staged brand-new file d.txt", "feature", &|p| { std::fs::write(p.join("d.txt"), "d\n").unwrap(); git(p, &["add", "d.txt"]); });
    scenario("unstaged deletion of b.txt", "feature", &|p| std::fs::remove_file(p.join("b.txt")).unwrap());
    scenario("target == current", "main", &|_| {});
    scenario("missing branch", "nope", &|_| {});
    // hooks and LFS-style filters
    let a = fixture(); let b = fixture();
    for p in [a.path(), b.path()] {
        let hook = p.join(".git/hooks/post-checkout");
        std::fs::write(&hook, "#!/bin/sh\ntouch hook-ran\n").unwrap();
        use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    git(a.path(), &["checkout", "feature"]); let _ = git2_checkout(b.path(), "feature");
    println!("-- post-checkout hook: cli ran={} git2 ran={}", a.path().join("hook-ran").exists(), b.path().join("hook-ran").exists());
    // smudge filter (what git-lfs uses)
    let a = fixture(); let b = fixture();
    for p in [a.path(), b.path()] {
        std::fs::write(p.join(".git/info/attributes"), "a.txt filter=up\n").unwrap();
        git(p, &["config", "filter.up.smudge", "tr a-z A-Z"]); git(p, &["config", "filter.up.clean", "cat"]);
    }
    git(a.path(), &["checkout", "feature"]); let _ = git2_checkout(b.path(), "feature");
    println!("-- external smudge filter: cli a.txt={:?} git2 a.txt={:?}", std::fs::read_to_string(a.path().join("a.txt")).unwrap().trim(), std::fs::read_to_string(b.path().join("a.txt")).unwrap().trim());
    let _: PathBuf = PathBuf::new();
}
```
