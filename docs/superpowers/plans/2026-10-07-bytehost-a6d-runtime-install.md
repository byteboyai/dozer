# bytehost A6d:运行时安装(uv/Python、Node)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 规格 A9 的落地:用户在 设置 → 应用 页点「安装」,看到**要下载什么(来源 URL、固定版本、SHA-256、装到哪)**并显式确认,dozerd 把 Node、uv(+ 由 uv 管理的 Python)装到 bytehost 自己的目录;装好之后进程型应用的 `RuntimeResolver` 优先用它们;可以卸载。Docker/Colima 仍只探测。

**Architecture:** `bytehost-apps` 新增 `runtime::managed`(`server` feature、unix):`pins`(版本/URL/校验和的**代码内常量表**,由脚本生成、人审后提交)、`store`(`<root>/runtimes/<name>/<version>/` 布局)、`fetch`(`Fetcher`/`Extractor` trait + 用系统 `curl`/`tar` 的实现,测试换假的)、`install`(下载 → 校验 SHA-256 → 解压到 staging → 路径与符号链接越界检查 → 试跑 `--version` → 原子改名)、`manager`(`RuntimeManager`:`plan` → 用户批准 → `start_install` 起后台线程,`status` 给进度,**安装时重新计算计划并核对**,同 `ApprovedInstallPlan::verify` 的思路)、`resolve`(`ManagedResolver` + `ChainResolver`:受管版本优先,系统兜底)。线上类型进 `proto`,dozerd 的 `app_service` 承接,GUI 设置页加审批与进度。

**Tech Stack:** Rust;下载与解压**调系统 `/usr/bin/curl`、`/usr/bin/tar`**(macOS 自带;不给 dozerd 引入 TLS/HTTP 客户端依赖,与"核心不依赖 Node/Python"一致;校验和我们自己用已有的 `sha2` 算);测试用本地文件当"下载源"。

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md` §6.2、A9;前置 `…-a6c-process-apps-wiring.md`(`RuntimeResolver`、`AppManager::with_resolver`)。

## Global Constraints

- **版本固定、校验和强制、来源可审、不静默下载**(规格 §6.2 原文):每个可装物只有一个 `Pin { runtime, version, target, url, sha256, … }`;校验不过**一律删掉已下载文件并失败**,没有"跳过校验"的开关;下载只在用户确认之后发生,确认的内容就是 dozerd 重新算出来的那份计划(不一致 → 拒绝)。
- **只信 HTTPS**:`curl --proto =https --proto-redir =https --tlsv1.2 --fail --location --silent --show-error --max-filesize 400000000`;URL 必须以 `https://` 开头(`Pin` 构造时断言)。
- **来源只有官方源**:Node 来自 `https://nodejs.org/dist/v<ver>/node-v<ver>-darwin-<arch>.tar.gz`(校验和取自同目录 `SHASUMS256.txt`);uv 来自 `https://github.com/astral-sh/uv/releases/download/<ver>/uv-<triple>.tar.gz`(校验和取自同名 `.sha256` 资产)。**Python 本身不由我们下载**:由已装好的 uv 执行 `uv python install <ver>`(来源 python-build-standalone,哈希由 uv 内置校验)——安装计划里必须把这一点**如实写明**(信任的是 uv 的内置哈希,而不是我们固定的校验和)。
- **校验和不许凭记忆或让模型转述**:`Pin` 里的 SHA-256 只能由 Task 1 的脚本从官方源**直接 `curl` 下来**生成(能验 GPG 签名就验),人工对一遍官方页面后提交。执行本计划的 agent 不得手写任何哈希。
- **解压不能越界**:先 `tar -tf` 列出条目,拒绝绝对路径、含 `..` 的条目;解压后遍历,**任何符号链接按字面解析后必须仍在 staging 目录内**(Node 包里 `bin/npm -> ../lib/node_modules/npm/bin/npm-cli.js` 这类相对链接合法);解压用 `--no-same-owner`。
- **装到 bytehost 自己的目录,不动系统环境**:`<bytehost root>/runtimes/{node,uv,python}/<version>/`;不写 `~/.nvm`、不改 shell 配置、不放进用户 `PATH`。uv 管理 Python 时设 `UV_PYTHON_INSTALL_DIR`、`UV_CACHE_DIR` 指向上述目录(`UV_PYTHON_PREFERENCE=only-managed`)。
- **原子性**:安装全程在 `runtimes/.staging-<uuid>/` 里做,试跑通过后一次 `rename` 落位;失败/取消/崩溃都不留半成品(下次启动清扫 `.staging-*`)。
- **同一运行时同一时刻只有一个安装任务**;安装进行中再请求同一运行时 → `Conflict`。
- **不碰正在被用的版本**:卸载一个受管版本不检查应用是否在用——卸载后依赖它的应用下次启动会得到 `RuntimeUnavailable`(A6c 已有的失败路径);界面的确认文案要写明这一点。
- **平台**:只发 macOS(`aarch64-apple-darwin`、`x86_64-apple-darwin`);其他目标 `pin_for` 返回 `None`,界面显示"此平台暂不支持自动安装"。
- `bytehost-apps` 默认 feature 依赖不变;`scripts/check-bytehost-apps-deps.sh` 必须通过;`dozerd`/`dozer-app` 的日志走 `dozer_core::log`;`cargo clippy -p bytehost-apps -p dozerd -p dozer-app --all-targets` 无新警告。
- 线上协议**只往后加**:新增的 `AppRequest`/`AppReply` 变体、`RuntimeProbe` 新字段(`#[serde(default)]`)都要有 serde 往返测试,旧 JSON 仍能解析。

## Review Focus

- **哈希不对必须失败且清理干净**(下载了 400MB 假包也一样):校验失败后 `runtimes/` 里没有任何新文件(Task 2 测试)。
- **恶意/损坏的压缩包**:`../` 条目、绝对路径、指向 staging 外的符号链接、解压后没有预期可执行文件、可执行文件试跑 `--version` 失败——全部失败且不落位(Task 2 逐条测试)。
- **批准的不是新算的**:客户端发来的计划与服务端重新计算的不一致(URL/哈希/版本被改)→ 拒绝并且**不发起任何下载**(Task 3、4 测试)。
- **下载中途取消/dozerd 退出**:`suspend_all`/shutdown 要能中止下载线程并清理 staging,不能把 dozerd 的退出拖住(Task 3)。
- **没有网络/被代理拦截**:`curl` 失败 → 任务 `failed` 带可读原因,界面可重试,不卡在"下载中"(Task 3 用假 `Fetcher` 返回错误;Task 6 有一个手动运行的真实网络冒烟)。
- **两个架构**:`pin_for` 在 arm64 与 x86_64 各有一份,不会把 arm64 的包装到 Intel 机器上(Task 1 测试按 target 参数化)。

## 文件结构

- 新增 `crates/bytehost-apps/src/runtime/managed/{mod.rs,pins.rs,store.rs,fetch.rs,install.rs,manager.rs,resolve.rs}`
- 新增 `scripts/bytehost/pin-runtimes.sh` — 从官方源生成 `pins.rs` 的数据段
- 修改 `crates/bytehost-apps/src/{proto.rs,runtime/mod.rs}`、`crates/dozer-client/src/lib.rs`(或其 app 模块)
- 修改 `crates/dozerd/src/app_service.rs`、`crates/dozer-app/src/extensions/{settings_apps.rs,settings.rs}`
- 修改 `CLAUDE.md`、`docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`(§6.2 标注已落地)

---

### Task 1: 固定版本表(`pins.rs`)与存储布局(`store.rs`)+ 生成脚本

**Files:** Create `runtime/managed/{mod.rs,pins.rs,store.rs}`、`scripts/bytehost/pin-runtimes.sh`;Modify `runtime/mod.rs`(`pub mod managed;`)、`proto.rs`(只加 `ManagedRuntime` 枚举及其 serde 往返测试)。

**Interfaces:**
- Produces:
  ```rust
  // 定义在 `proto.rs`(默认 feature、只依赖 serde,线上要用),`managed` 里 `pub use crate::proto::ManagedRuntime;`
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
  #[serde(rename_all = "snake_case")]
  pub enum ManagedRuntime { Node, Python }          // Python = uv + 它管理的 CPython
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum Target { Aarch64Apple, X86_64Apple }
  impl Target { pub fn current() -> Option<Target> }   // 按 cfg!(target_arch) + cfg!(target_os="macos")
  pub struct Pin { pub name: &'static str /*"node"|"uv"*/, pub version: &'static str, pub target: Target,
                   pub url: &'static str, pub sha256: &'static str, pub strip_components: u32,
                   pub bin_rel: &'static str /* 解压根下可执行文件的相对路径,如 "bin/node" */ }
  pub const PYTHON_VERSION: &str = "3.13";           // 传给 `uv python install`
  pub fn pin_for(name: &str, target: Target) -> Option<&'static Pin>;
  pub struct RuntimeStore { /* root: PathBuf (= <bytehost root>/runtimes) */ }
  impl RuntimeStore {
      pub fn new(root: impl Into<PathBuf>) -> Self;
      pub fn version_dir(&self, name: &str, version: &str) -> PathBuf;   // <root>/<name>/<version>
      pub fn installed(&self, name: &str) -> Vec<String>;                 // 目录名,版本号降序(按语义比较,失败退字符串)
      pub fn staging_dir(&self) -> PathBuf;                               // <root>/.staging-<uuid>(不创建)
      pub fn sweep_staging(&self);
      pub fn remove(&self, name: &str, version: &str) -> io::Result<()>;  // 幂等
  }
  ```

- [ ] **Step 1: 写生成脚本** `scripts/bytehost/pin-runtimes.sh <node-version> <uv-version>`:对两个 target 各自 `curl -fsSL` 官方校验和来源(Node:`https://nodejs.org/dist/v$N/SHASUMS256.txt`,取 `node-v$N-darwin-{arm64,x64}.tar.gz` 两行;uv:`https://github.com/astral-sh/uv/releases/download/$U/uv-{aarch64,x86_64}-apple-darwin.tar.gz.sha256`);Node 若本机有 `gpg` 且能取到 `SHASUMS256.txt.sig` 则验签,**验不了就在输出里打印醒目的 `UNVERIFIED-SIGNATURE` 提示**;最后把四条 `Pin` 的 Rust 字面量打印到 stdout。脚本 `set -euo pipefail`,任何一步失败整体失败,**不产出部分结果**。
- [ ] **Step 2: 运行脚本生成数据**:Node 取当前 24.x LTS 最新补丁版、uv 取最新稳定版(执行时查官方发布页确定,**把选定的两个版本号写进提交信息**)。人工打开官方 `SHASUMS256.txt`/`.sha256` 页面逐条比对脚本输出,**比对结果写进提交信息**("校验和已与官方页面逐条核对")。
- [ ] **Step 3: 写失败测试**(`pins.rs`、`store.rs` 的 `mod tests`)

```rust
// pins.rs
#[test]
fn every_pin_is_https_and_has_a_64_hex_checksum_and_a_relative_bin() {
    for t in [Target::Aarch64Apple, Target::X86_64Apple] {
        for name in ["node", "uv"] {
            let p = pin_for(name, t).unwrap_or_else(|| panic!("{name} {t:?}"));
            assert!(p.url.starts_with("https://"), "{}", p.url);
            assert_eq!(p.sha256.len(), 64);
            assert!(p.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
            assert!(!p.bin_rel.starts_with('/') && !p.bin_rel.contains(".."));
            assert_eq!(p.target, t);
        }
    }
}

#[test]
fn the_two_architectures_never_share_a_url_or_a_checksum() {
    for name in ["node", "uv"] {
        let a = pin_for(name, Target::Aarch64Apple).unwrap();
        let x = pin_for(name, Target::X86_64Apple).unwrap();
        assert_ne!(a.url, x.url);
        assert_ne!(a.sha256, x.sha256);
        assert_eq!(a.version, x.version, "两个架构装同一个版本");
    }
}

#[test]
fn urls_match_the_official_sources() {
    let n = pin_for("node", Target::Aarch64Apple).unwrap();
    assert!(n.url.starts_with("https://nodejs.org/dist/v"));
    let u = pin_for("uv", Target::Aarch64Apple).unwrap();
    assert!(u.url.starts_with("https://github.com/astral-sh/uv/releases/download/"));
}

// store.rs
#[test]
fn installed_versions_are_listed_newest_first_and_ignore_staging_and_files() {
    let d = tempfile::tempdir().unwrap();
    let s = RuntimeStore::new(d.path());
    for v in ["22.1.0", "24.14.0", "24.9.0"] {
        std::fs::create_dir_all(s.version_dir("node", v)).unwrap();
    }
    std::fs::create_dir_all(d.path().join(".staging-x")).unwrap();
    std::fs::write(d.path().join("node").join("stray.txt"), "x").unwrap();
    assert_eq!(s.installed("node"), vec!["24.14.0", "24.9.0", "22.1.0"]);
    assert!(s.installed("uv").is_empty());
}

#[test]
fn remove_is_idempotent_and_sweep_clears_only_staging() {
    let d = tempfile::tempdir().unwrap();
    let s = RuntimeStore::new(d.path());
    std::fs::create_dir_all(s.version_dir("uv", "1.0.0")).unwrap();
    std::fs::create_dir_all(d.path().join(".staging-a/deep")).unwrap();
    s.remove("uv", "1.0.0").unwrap();
    s.remove("uv", "1.0.0").unwrap();
    s.sweep_staging();
    assert!(!d.path().join(".staging-a").exists());
    assert!(s.installed("uv").is_empty());
}

#[test]
fn version_names_that_could_escape_the_store_are_rejected() {
    let s = RuntimeStore::new("/r");
    for bad in ["..", "../x", "a/b", "", "."] {
        assert!(s.try_version_dir("node", bad).is_none(), "{bad}");
    }
}
```
  (给 `RuntimeStore` 加 `try_version_dir(name, version) -> Option<PathBuf>`:`name`/`version` 只含 `[A-Za-z0-9._-]` 且不是 `.`/`..`;`version_dir`/`remove` 内部都经它,非法时 `remove` 返回 `InvalidInput`。)
- [ ] **Step 4: 确认失败 → 实现 → 通过**(`cargo test -p bytehost-apps --all-features runtime::managed`)。
- [ ] **Step 5: Commit** — `feat(bytehost-apps): pinned runtime table (generated from official sources) and runtime store layout (A6d task 1)`;提交信息正文写明选定版本与核对结论。

---

### Task 2: 下载、校验、解压、落位(机制,假源测试)

**Files:** Create `runtime/managed/{fetch.rs,install.rs}`。

**Interfaces:**
- Consumes:Task 1 的 `Pin`/`RuntimeStore`。
- Produces:
  ```rust
  pub trait Fetcher: Send + Sync {
      /// 把 `url` 下载到 `dest`;`on_progress(done, total)`;`cancel` 置位时尽快中止并返回 Err(Interrupted)。
      fn fetch(&self, url: &str, dest: &Path, on_progress: &mut dyn FnMut(u64, Option<u64>), cancel: &AtomicBool) -> io::Result<()>;
  }
  pub trait Archive: Send + Sync {
      fn list(&self, archive: &Path) -> io::Result<Vec<String>>;
      fn extract(&self, archive: &Path, into: &Path, strip_components: u32) -> io::Result<()>;
  }
  pub struct CurlFetcher;   // 调 /usr/bin/curl,轮询子进程以响应 cancel,进度来自 `-o` 文件大小与 `--head` 取得的长度(取不到则 total=None)
  pub struct TarArchive;    // 调 /usr/bin/tar -tf / -xf --no-same-owner --strip-components
  #[derive(Debug)] pub enum InstallError { Download(String), ChecksumMismatch{ expected: String, actual: String }, UnsafeArchive(String), MissingBinary(String), SmokeTestFailed(String), Cancelled, Io(io::Error) }
  pub struct Installer<'a> { pub store: &'a RuntimeStore, pub fetcher: &'a dyn Fetcher, pub archive: &'a dyn Archive }
  impl Installer<'_> {
      /// 下载 `pin.url` → 校验 → 解压 → 安全检查 → 试跑 → 落位到 `store.version_dir(pin.name, pin.version)`。
      pub fn install(&self, pin: &Pin, on_progress: &mut dyn FnMut(Phase, u64, Option<u64>), cancel: &AtomicBool) -> Result<PathBuf, InstallError>;
  }
  pub enum Phase { Downloading, Verifying, Extracting, Checking }
  ```
  `install` 的 `smoke` 步骤:运行 `<staging>/<pin.bin_rel> --version`(`std::process::Command`,5s 超时,清空环境只留 `PATH=/usr/bin:/bin`),退出码 0 才算过;结果输出不信任、不解析。

- [ ] **Step 1: 写失败测试**(`install.rs` 的 `mod tests`;用 `FileFetcher`(测试里实现 `Fetcher`:把预先放在 tempdir 里的文件拷到 `dest`,可配置先 sleep/返回错误)和真的 `TarArchive`(系统 `tar`,测试里用 `tar -czf` 现造小压缩包))。辅助 `make_tar(dir, entries)` 造出含可执行脚本 `bin/tool`(`#!/bin/sh\necho v1`,0755)的 `.tar.gz`,并返回其 sha256(`sha256_hex`)。`pin(url, sha, strip)` 构造测试用 `Pin`(`name: "tool"`, `bin_rel: "bin/tool"`)——`Pin` 的字段是 `&'static str`,测试里用 `Box::leak` 造。要写的测试:
  1. `a_valid_archive_is_installed_atomically_and_smoke_tested`:落位目录存在且 `bin/tool` 可执行,`staging` 里无残留,进度回调依次经过 Downloading→Verifying→Extracting→Checking。
  2. `a_wrong_checksum_fails_and_leaves_nothing_behind`:`sha256` 改一位 → `ChecksumMismatch{expected, actual}`;`runtimes/` 下除空的 store 根外**没有任何文件/目录**(含下载的归档)。
  3. `path_traversal_entries_are_refused`:归档含 `../evil`(用 `tar --transform`/`-P` 造,或手写 tar 头:用 Rust 的 `std` 手工写 512 字节 ustar 头最稳,封装成 `raw_tar(entries: &[(&str, &[u8], u8 /*typeflag*/ , &str /*linkname*/)])` 辅助)→ `UnsafeArchive`,且 store 外(tempdir 父级)没有 `evil` 文件;同理绝对路径 `/tmp/x`。
  4. `a_symlink_pointing_outside_the_staging_dir_is_refused_but_a_relative_inner_link_is_fine`:`bin/a -> ../../../etc/passwd` 拒绝;`bin/b -> ../lib/real` 通过。
  5. `an_archive_without_the_expected_binary_is_refused`(`MissingBinary`)、`a_binary_that_fails_its_smoke_test_is_refused`(脚本 `exit 1` → `SmokeTestFailed`),两者都不落位。
  6. `cancelling_during_the_download_aborts_and_cleans_up`:`FileFetcher` 在下载中置位 `cancel` → `Cancelled`,无残留。
  7. `a_failing_download_is_reported_with_its_reason_and_cleans_up`(`Download("…")`)。
  8. `reinstalling_an_existing_version_replaces_it_atomically_or_is_refused`:约定为**拒绝**(`Io(AlreadyExists)`):已装版本不覆盖,要换就先卸载。
- [ ] **Step 2: 确认失败**。
- [ ] **Step 3: 实现。** `CurlFetcher` 命令行见 Global Constraints;子进程用 `Command::spawn`,每 100ms `try_wait` 并检查 `cancel`(置位则 `kill` + `wait`,返回 `Interrupted`);非 0 退出把 curl 的 stderr 最后一行放进 `Download(..)`。越界检查写成纯函数 `fn entry_is_safe(name: &str) -> bool` 与 `fn link_stays_inside(root: &Path, link: &Path) -> bool`(字面规范化,不跟随文件系统),各有表驱动单测。
- [ ] **Step 4: 通过**(`cargo test -p bytehost-apps --all-features runtime::managed`,连跑 3 遍)。
- [ ] **Step 5: Commit** — `feat(bytehost-apps): runtime installer — checksum-verified download, safe extraction, smoke test, atomic placement (A6d task 2)`。

---

### Task 3: `RuntimeManager`(计划/批准/后台任务/Python via uv)与 `ManagedResolver`

**Files:** Create `runtime/managed/{manager.rs,resolve.rs}`;Modify `runtime/managed/mod.rs`、`runtime/mod.rs`。

**Interfaces:**
- Consumes:Task 1–2;A6c 的 `RuntimeResolver`/`SystemResolver`/`Resolved`/`ResolveError`;`proto` 里 Task 4 才加的线上类型**不在本任务**——本任务用同形的库内类型,Task 4 再 `pub use`/映射。
- Produces:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct DownloadItem { pub what: String, pub url: String, pub sha256: Option<String> /* uv 管理的 Python 为 None */, pub note: String }
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct InstallPlanRt { pub runtime: ManagedRuntime, pub versions: Vec<(String, String)> /*(name, version)*/, pub downloads: Vec<DownloadItem>, pub dest: String, pub will_do: Vec<String> }
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct Job { pub runtime: ManagedRuntime, pub phase: String, pub done: u64, pub total: Option<u64>, pub failed: Option<String>, pub finished: bool }
  #[derive(Debug)] pub enum RtError { Unsupported(String), Conflict(String), PlanChanged, Io(io::Error) }
  pub struct RuntimeManager { /* store, fetcher: Arc<dyn Fetcher>, archive: Arc<dyn Archive>, target: Option<Target>, jobs: Mutex<HashMap<ManagedRuntime, JobEntry>> */ }
  impl RuntimeManager {
      pub fn new(root: impl Into<PathBuf>) -> Self;                       // 真 curl/tar
      pub fn with_parts(root, fetcher: Arc<dyn Fetcher>, archive: Arc<dyn Archive>, target: Option<Target>, uv_runner: Arc<dyn UvRunner>) -> Self;
      pub fn plan(&self, rt: ManagedRuntime) -> Result<InstallPlanRt, RtError>;
      /// 重新计算计划并与 `approved` 逐字段比较;一致才起后台线程。已有进行中的任务 → `Conflict`。
      pub fn start_install(&self, approved: &InstallPlanRt) -> Result<(), RtError>;
      pub fn jobs(&self) -> Vec<Job>;
      pub fn installed(&self, rt: ManagedRuntime) -> Vec<String>;
      pub fn uninstall(&self, rt: ManagedRuntime, version: &str) -> Result<(), RtError>;
      pub fn cancel_all_and_join(&self);                                   // dozerd 退出时调
      pub fn store(&self) -> &RuntimeStore;
  }
  pub trait UvRunner: Send + Sync { fn python_install(&self, uv: &Path, version: &str, install_dir: &Path, cache_dir: &Path, cancel: &AtomicBool) -> io::Result<()>; }
  pub struct ManagedResolver { store: RuntimeStore }                       // impl RuntimeResolver
  pub struct ChainResolver(pub Vec<Arc<dyn RuntimeResolver>>);             // 依次尝试,第一个 Ok 即用;全失败返回最后一个 NotInstalled
  ```
  行为:
  - `plan(Node)`:一条 `DownloadItem`(node pin,`sha256: Some`);`dest = <root>/node/<version>`;`will_do` 含"下载并校验 SHA-256"、"解压到 …"、"不修改系统环境与 shell 配置"。
  - `plan(Python)`:两条:uv(`Some(sha)`)与 `Python <PYTHON_VERSION>`(`url: "由 uv 从 python-build-standalone 下载"`,`sha256: None`,`note: "哈希由 uv 内置校验,不是 bytehost 固定的校验和"`);`will_do` 含"运行 uv python install 3.13(UV_PYTHON_INSTALL_DIR=…)"。目标不受支持(`Target::current()` 为 `None`)→ `Unsupported`。
  - 后台线程(Python):先装 uv(`Installer`,若该 uv 版本已在 store 就跳过),再 `UvRunner::python_install(uv_bin, PYTHON_VERSION, <root>/python, <root>/cache/uv, cancel)`(真实现:`Command` 起 `uv python install <ver>`,环境 `PATH=/usr/bin:/bin`、`HOME`、`UV_PYTHON_INSTALL_DIR`、`UV_CACHE_DIR`、`UV_PYTHON_PREFERENCE=only-managed`、`UV_NO_CONFIG=1`,继承 `HTTPS_PROXY`/`HTTP_PROXY`/`ALL_PROXY`/`NO_PROXY` 以便走代理;轮询 `cancel`)。
  - `ManagedResolver::resolve`:`node`/`npm`/`npx` → 最新受管 Node 的 `bin/`;`uv` → 最新受管 uv 的可执行文件所在目录;`python`/`python3` → `<root>/python/cpython-*/bin/python3`(按目录名降序取第一个存在可执行文件的);裸名规则同 `SystemResolver`(含路径分隔符一律拒绝)。没装返回 `NotInstalled`。`path_dirs` 返回该可执行文件所在目录。
  - `cancel_all_and_join`:置位所有任务的 cancel、`join` 线程;之后 `sweep_staging`。

- [ ] **Step 1: 写失败测试**(假 `Fetcher`/`UvRunner`、真 `TarArchive`;`Target` 注入为 `Some(Aarch64Apple)`;`Pin` 用测试里造的假表需要 `pin_for` 可注入:`RuntimeManager::with_parts` 额外接受 `pins: &'static [Pin]` 参数——**加到 `with_parts` 签名里**)。要写的测试:
  1. `the_node_plan_lists_exactly_the_pinned_download_and_destination`。
  2. `the_python_plan_discloses_that_python_itself_is_verified_by_uv_not_by_a_pinned_checksum`(第二条 `sha256 == None` 且 `note` 含"uv 内置")。
  3. `an_unsupported_target_cannot_be_planned_or_installed`(`Target = None` → `Unsupported`)。
  4. `a_changed_plan_is_refused_and_nothing_is_downloaded`:改 `downloads[0].url`/`sha256`/`versions` 各一遍 → `PlanChanged`,假 `Fetcher` 的调用计数为 0。
  5. `an_approved_install_runs_in_the_background_and_reports_progress_then_finishes`:假 fetcher 分步报进度;轮询 `jobs()` 看到 `Downloading`(`done>0`)→ `finished == true && failed.is_none()`;`installed(Node)` 含该版本。
  6. `a_second_install_of_the_same_runtime_while_one_is_running_conflicts`。
  7. `a_failed_install_is_reported_and_a_retry_is_allowed`:第一次假 fetcher 返回错误 → `jobs()` 里 `failed = Some(原因)`;第二次 `start_install` 成功起新任务(旧失败记录被替换)。
  8. `python_install_runs_uv_after_installing_uv_and_skips_uv_when_already_present`(假 `UvRunner` 记录调用参数:`install_dir` 在 root 内、`version == "3.13"`;第二次安装 uv 的 fetcher 调用计数不增加)。
  9. `cancel_all_stops_a_running_download_and_cleans_staging`(假 fetcher 阻塞到 cancel;`cancel_all_and_join` 在 2s 内返回,`runtimes/` 下无 `.staging-*`)。
  10. `uninstall_removes_one_version_and_rejects_names_that_escape`。
  11. `the_managed_resolver_finds_node_npm_uv_and_python_and_refuses_paths`(在 tempdir 里手工摆好目录与可执行文件)。
  12. `the_chain_prefers_managed_but_falls_back_to_the_system`:Managed 没装 → 回落 `SystemResolver::with_dirs(..)`;装了 → 返回受管路径。
- [ ] **Step 2: 确认失败 → Step 3: 实现 → Step 4: 通过**。
- [ ] **Step 5: Commit** — `feat(bytehost-apps): RuntimeManager — approved-plan verification, background install jobs, uv-managed Python, managed resolver chain (A6d task 3)`。

---

### Task 4: 线上协议 + client + dozerd 接线

**Files:** Modify `crates/bytehost-apps/src/proto.rs`、`crates/dozer-client/src/lib.rs`(`app_*` 方法所在文件)、`crates/dozerd/src/app_service.rs`;(必要时 `crates/dozer-core` 的协议若对 `AppRequest` 做了穷举匹配——`grep -rn "AppRequest::" crates --include=*.rs` 逐处核对)。

**Interfaces:**
- Produces(`proto.rs`,默认 feature、只依赖 serde):
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
  pub enum ManagedRuntime { Node, Python }          // 从 proto 定义,managed 模块 `pub use`(默认 feature 下 proto 必须不依赖 server 代码)
  pub struct RuntimeDownload { pub what: String, pub url: String, pub sha256: Option<String>, pub note: String }
  pub struct RuntimeInstallPlan { pub runtime: ManagedRuntime, pub versions: Vec<(String,String)>, pub downloads: Vec<RuntimeDownload>, pub dest: String, pub will_do: Vec<String> }
  pub struct RuntimeJob { pub runtime: ManagedRuntime, pub phase: String, pub done: u64, pub total: Option<u64>, pub failed: Option<String>, pub finished: bool }
  // RuntimeProbe 新增(均 #[serde(default)]):
  //   pub managed: Vec<String>            // 已装的受管版本(新→旧)
  //   pub installable: bool               // 此平台有固定版本可装(node/python 为 true,docker 为 false)
  //   pub job: Option<RuntimeJob>
  // AppRequest 新增: RuntimePlan{runtime}, InstallRuntime{plan: Box<RuntimeInstallPlan>}, UninstallRuntime{runtime, version}
  // AppReply 新增:   RuntimePlan{plan: Box<RuntimeInstallPlan>};安装/卸载成功回 `Done`
  ```
  `ManagedRuntime` 与 `managed` 模块里 Task 1 的同名枚举**合并成一处**:定义在 `proto.rs`,`managed` 模块 `pub use crate::proto::ManagedRuntime`(Task 1 若已定义则移过来并改引用);`InstallPlanRt`/`Job` 同理改为直接使用 `proto` 里的 `RuntimeInstallPlan`/`RuntimeJob`(本任务把 Task 3 的库内类型替换成 proto 类型,测试随之改名)。
  - `dozer-client`:`app_runtime_plan(runtime) -> Result<RuntimeInstallPlan>`、`app_install_runtime(plan) -> Result<()>`、`app_uninstall_runtime(runtime, version) -> Result<()>`。
  - `app_service`:`State::Ready` 多持一个 `Arc<RuntimeManager>`(根目录 `<root>/runtimes`,启动时 `sweep_staging`);`AppManager` 用 `with_resolver(…, ChainResolver[ManagedResolver, SystemResolver])` 构造(**受管优先**);`ProbeRuntimes` 回复里对 `node`/`python` 填 `managed`/`installable`/`job`;`shutdown()` 先 `runtime_manager.cancel_all_and_join()`(在 `suspend_all` 之前)。失败映射:`Unsupported`→`AppErrorKind::Unsupported`,`Conflict`→`Conflict`,`PlanChanged`→`Rejected`,`Io`→`Internal`。

- [ ] **Step 1: 写失败测试**:`proto.rs`:新请求/应答与 `RuntimeProbe` 新字段的 serde 往返;**旧形状的 `RuntimeProbe` JSON(没有 `managed`/`installable`/`job`)仍能解析**。`app_service.rs`(用 `AppService` 的测试构造入口注入一个带假 `Fetcher`/真 `TarArchive` 的 `RuntimeManager`——给 `AppService::start_with` 加一个 `#[cfg(test)]` 的变体或让 `finish_start` 接受 `Arc<RuntimeManager>`):
  1. `a_runtime_install_goes_plan_approve_progress_installed_and_probes_show_it`:`RuntimePlan` → `InstallRuntime` → 轮询 `ProbeRuntimes` 直到 `job.finished` → `managed` 含版本。
  2. `a_forged_runtime_plan_is_rejected_without_downloading`(改 URL → `Rejected`,假 fetcher 计数 0)。
  3. `a_process_app_uses_the_managed_runtime_before_the_system_one`:装好受管 `python`(假 `UvRunner` 在目标目录放一个会起 HTTP 服务的 `python3` 包装脚本,内部转调系统 `python3`,并在 stdout/文件里留下"我是受管版本"的标记)→ 安装并启动一个 python 应用 → 标记文件存在。
  4. `shutdown_cancels_a_running_download`。
- [ ] **Step 2: 确认失败 → Step 3: 实现 → Step 4: 通过**(`cargo test -p bytehost-apps -p dozerd -p dozer-client`;`scripts/check-bytehost-apps-deps.sh`)。
- [ ] **Step 5: Commit** — `feat(dozerd): runtime install over the wire — plan/approve/progress, managed runtimes preferred (A6d task 4)`。

---

### Task 5: 设置页——审批与进度(纯状态机 + 视图)

**Files:** Modify `crates/dozer-app/src/extensions/settings_apps.rs`、`crates/dozer-app/src/extensions/settings.rs`(effect 执行)。

**Interfaces:**
- Consumes:Task 4 的 `RuntimeProbe` 新字段、`RuntimeInstallPlan`、`RuntimeJob`、client 的三个新方法。
- Produces(状态机新增):
  ```rust
  // Flow 新增
  RuntimeReviewing { plan: Box<RuntimeInstallPlan> },       // 展示来源/校验和/目标目录,等批准
  RuntimePlanning { runtime: ManagedRuntime },
  ConfirmRuntimeUninstall { runtime: ManagedRuntime, version: String },
  // Message 新增
  RuntimeInstallClicked(ManagedRuntime), RuntimePlanLoaded(Result<Box<RuntimeInstallPlan>, Failure>),
  RuntimeApproveClicked, RuntimeInstallStarted(Result<(), Failure>),
  RuntimeUninstallClicked(ManagedRuntime, String), RuntimeUninstallConfirmed, RuntimeUninstallDone(Result<(), Failure>),
  PollProbes,                                                // 安装进行中的定时刷新
  // Effect 新增
  RuntimePlan(ManagedRuntime), InstallRuntime(Box<RuntimeInstallPlan>), UninstallRuntime(ManagedRuntime, String), ProbeAfter(std::time::Duration),
  // 纯函数
  pub fn runtime_rows(probes: &[RuntimeProbe]) -> Vec<RuntimeRow>   // 每个运行时一行:名称、状态文案、受管版本、按钮可用性、进度文案
  pub fn plan_lines(plan: &RuntimeInstallPlan) -> Vec<(String, String)>   // 审批页逐行:来源、固定版本、SHA-256(None 显示"由 uv 内置哈希校验")、安装位置
  ```
  行为:
  - `RuntimeInstallClicked(rt)` → `Flow::RuntimePlanning` + `Effect::RuntimePlan(rt)`;`RuntimePlanLoaded(Ok)` → `RuntimeReviewing`;`Err` → `Flow::Failed`(流程内失败,与既有一致)。
  - `RuntimeApproveClicked` 只在 `RuntimeReviewing` 有效 → `Effect::InstallRuntime(plan)`(**原封不动把展示的那份计划发回去**)并回到 `Idle`、`Effect::ProbeAfter(500ms)`;`RuntimeInstallStarted(Err)` → `Toast`(失败)。
  - `ProbesLoaded` 之后:只要任一 `probe.job` 存在且 `!finished` → 再发一个 `Effect::ProbeAfter(1s)`;`finished && failed.is_none()` 的任务出现一次 → 一条成功 Toast(用 `push_keyed` 去重,key 含运行时名)并 `HostChanged`;`failed = Some(why)` → 失败 Toast(同样去重)。
  - 卸载受管版本:确认页文案写明"依赖它的应用下次启动会因运行时缺失而失败"。
  - 视图:运行时行里,未装受管版本且 `installable` 的显示「安装…」;已装的显示版本与「卸载」;进度行显示 `phase` 与百分比(`total` 为 `None` 时只显示已下载 MB);`installable == false` 的运行时(docker)只显示状态,**不出现任何安装按钮**。审批页必须逐行展示 `plan_lines`,按钮「下载并安装」「取消」。
  - `settings.rs` 的 effect 执行:`ProbeAfter(d)` → `handle.spawn(async move { tokio::time::sleep(d).await; send(M::PollProbes) })`;`PollProbes` 的 `update` 返回 `[Effect::Probe]`(且在 `Flow` 之外不改变任何状态);其余三个新 effect 调 client 新方法,结果走对应 `Message`。

- [ ] **Step 1: 写失败测试**(沿用该文件现有测试风格:构造 `State`、喂 `Message`、断言 `Effect` 与状态):
  1. `clicking_install_plans_then_reviewing_shows_every_download_with_its_checksum`:`plan_lines` 含 URL、版本、完整 64 位 SHA-256、目标目录;`sha256 == None` 的那条显示"由 uv 内置哈希校验"。
  2. `approving_sends_back_exactly_the_plan_that_was_shown_and_starts_polling`。
  3. `approve_outside_reviewing_does_nothing`(`Idle`/`RuntimePlanning` 下收到 `RuntimeApproveClicked` → 无 effect)。
  4. `polling_continues_only_while_a_job_is_unfinished_and_stops_after`。
  5. `a_finished_job_toasts_once_and_a_failed_one_toasts_its_reason_once`(多次 `ProbesLoaded` 同一结果,Toast 只发一次——用 key 去重 + 状态里记已报告的任务)。
  6. `docker_never_offers_install`、`an_installed_managed_version_offers_uninstall_with_the_warning`。
  7. `a_plan_failure_stays_in_the_flow_not_a_toast`。
  8. `runtime_rows_show_progress_percent_or_megabytes`(表驱动)。
- [ ] **Step 2: 确认失败 → Step 3: 实现 → Step 4: 通过**(`cargo test -p dozer-app settings_apps`、`cargo clippy -p dozer-app --all-targets`)。
- [ ] **Step 5: 手动验收**(记录在提交信息里;需要 GUI):`cargo run -p dozer-app` → 设置 → 应用 → Node「安装…」→ 审批页的 URL/SHA-256 与 `scripts/bytehost/pin-runtimes.sh` 输出一致 → 安装 → 进度走完 → 行里出现版本 → 装一个 `node` 应用能起来 → 卸载受管版本后该应用启动得到"运行时 node 未安装"。
- [ ] **Step 6: Commit** — `feat(dozer-app): settings — runtime install review/progress/uninstall (A6d task 5)`。

---

### Task 6: 真实网络冒烟与文档

**Files:** Create `crates/bytehost-apps/tests/runtime_install_live.rs`(`#[ignore]`);Modify `CLAUDE.md`、规格 §6.2。

- [ ] **Step 1: 冒烟测试**(默认不跑):`cargo test -p bytehost-apps --all-features --test runtime_install_live -- --ignored`:用真 `CurlFetcher`/`TarArchive`、tempdir 作 root,依次装 Node 与 Python(uv + `uv python install`),断言 `node --version`/`python3 --version` 能跑、受管解析器能找到它们;结果(耗时、版本号)记入提交信息。没有网络时测试**失败而不是跳过**(它只在人工要求时运行)。
- [ ] **Step 2: 文档**:`CLAUDE.md` 的 `bytehost-apps` 一行补:运行时安装已落地(A6d)——`runtime::managed`、固定版本表由 `scripts/bytehost/pin-runtimes.sh` 从官方源生成(**不得手写哈希**)、下载走系统 `curl`/`tar`、Python 由 uv 管理且哈希由 uv 内置校验(计划里如实披露)、受管运行时优先于系统、安装计划在服务端重新计算并核对;规格 §6.2 与 A9 行标注"已落地(A6d)"。
- [ ] **Step 3: 全量**:`cargo test -p bytehost-apps -p dozerd -p dozer-client -p dozer-app`、`cargo clippy --all-targets`、`scripts/check-bytehost-apps-deps.sh`、`scripts/check-log-scope.sh`。
- [ ] **Step 4: Commit** — `docs(bytehost): runtime install landed (A6d task 6)`。

---

## 已知局限(写在这里,不是缺陷)

- **只有最新固定版本**:每个运行时只固定一个版本;升级固定版本要改 `pins.rs`(重跑脚本、人审)再发版。不支持用户任选版本。
- **Python 的信任根是 uv 的内置哈希**,不是 bytehost 固定的校验和;计划里如实披露。
- **没有自动更新/漏洞通知**;受管运行时不会随系统更新。
- **应用清单里的 `node`/`python` 版本要求(`manifest.node`/`manifest.python`)仍不读**:用解析到的(受管最新或系统)版本。
- **下载走系统 `curl`**:代理取决于 dozerd 进程环境里的 `HTTPS_PROXY` 等变量;GUI 里探测到的代理不会自动传给 dozerd。
- **卸载不检查是否被应用使用**;依赖它的应用会在下次启动时得到 `RuntimeUnavailable`。
- **一次只装一个运行时的一个任务**;没有下载断点续传。
- **只有 macOS**;其他平台 `installable = false`。

## Self-Review(写计划时已核对)

- **Spec 覆盖**:A9"可装 uv/Python、Node(固定版本+校验和,装到 bytehost 自己的目录,显式确认);Docker 只探测"→ Task 1(固定版本/校验和)、Task 2(安全落位)、Task 3(显式批准 = 服务端重算核对)、Task 5(确认界面,docker 无安装入口);§6.2"下载是安全敏感操作:版本固定、校验和强制、来源可审、下载在设置界面里明示;不静默下载"→ Global Constraints 与 Task 3/5 测试;§6.2 `RuntimeManager`(探测、安装、列出版本、卸载)→ Task 3(`installed`/`uninstall`)+ 既有 `probe`。A6c 留下的"运行时二进制解析"由 `ChainResolver` 接入(Task 4)。
- **占位符**:无手写哈希/版本占位——哈希只来自 Task 1 的脚本,执行时确定版本,计划里明确禁止 agent 手写;`pins.rs` 数据段由脚本输出,不在计划里内联。
- **类型一致**:`ManagedRuntime` 统一定义在 `proto.rs`(Task 4 把 Task 1/3 里的同名/同形类型合并过去并写明);`Pin`/`RuntimeStore`/`Installer`/`Fetcher`/`Archive` 在 Task 1–2 定义,Task 3 按同名使用;`with_parts` 的签名在 Task 3 里一次写全(含 `pins`)。
