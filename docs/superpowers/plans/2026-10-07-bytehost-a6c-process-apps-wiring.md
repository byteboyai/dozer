# bytehost A6c:进程型应用接入 AppManager / dozerd Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `node`/`python` 类型的应用真的能装、能启动、能停止:`install_plan` 不再拒绝它们(并校验端口变量名/命令/lockfile),`start` 起一条**监管线程**(装依赖 → 起进程 → 健康探测 → 注册 gateway 上游 → 崩溃按退避重启/放弃),状态与端点事件照常发出,`stop`/`uninstall`/`suspend_all` 能干净地收掉进程,dozerd 重启时 `reconcile` 先回收孤儿再按 `desired` 拉起。

**Architecture:** 把 `AppManager` 的共享部分(注册表、gateway、事件、单写者锁、关闭标志)抽成 `Arc<Core>`,`AppManager` 持有它并 `Deref` 到它(**现有方法体与测试不改**)。新增 `supervisor.rs`:一个与 manager 解耦的监管线程函数 `run`,通过 `Transitions` trait 回报阶段变化(返回 `false` = 这次监管已被取消,线程立刻收尾、不再写任何状态);manager 提供 `Transitions` 的实现(持锁、核对取消标志、写 `AppRecord`、发事件、增删 gateway 上游)。运行时二进制由 `RuntimeResolver`(`SystemResolver`)解析成绝对路径并给子进程一个可靠的 `PATH`,不依赖 dozerd 继承的 `PATH`。**不做**运行时安装(A6d)、容器、日志查看界面。

**Tech Stack:** Rust、`std::thread`(监管线程,与 A6a 的阻塞式 `process` 模块一致)、`bytehost-apps` 的 `server` feature、测试用真的子进程(`python3`/`sh`;缺 `python3` 的机器上相应测试跳过)。

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`(§4.4 生命周期、§6.1 进程所有权、§6.2 运行时来源)。前置:`docs/superpowers/plans/2026-10-07-bytehost-a6a-process-supervisor.md`(监管核心)、`…-a6b-gateway-reverse-proxy.md`(`Gateway::add_upstream`)。

## A6 切分

| 片 | 状态 |
|---|---|
| A6a 监管核心 | 已合并 |
| A6b gateway 反向代理 | 已合并 |
| **A6c(本计划)** | 可执行 |
| A6d 运行时安装(uv/Node 下载、校验和、Settings 界面) | 待写 |

## Global Constraints

- **单写者**:所有写 `AppRecord` 的路径(含监管线程的回写)都在 `Core::guard()` 里做,且**先落盘再发事件**(与现有 `set_observed` 一致)。
- **监管线程绝不在"被取消"之后写状态**:每次回写都在拿到锁之后核对该次监管的 `cancel` 标志;取消了就什么都不写、返回 `false`。
- **监管线程拿锁用"尝试 + 查取消"循环,绝不阻塞等锁**:`stop_locked`/`suspend_all` 持着锁 `join` 监管线程,线程若阻塞等锁就是死锁。
- **子进程环境仍是白名单**(`process::env::build_env`);`PATH` 由解析器给出的目录 + `/usr/bin:/bin` 重建,**不用** dozerd 继承的 `PATH`(GUI 拉起的 dozerd 往往只有 `/usr/bin:/bin`)。应用的 npm/uv 缓存指向应用自己的 `cache/` 目录(`npm_config_cache`、`UV_CACHE_DIR`),不写用户主目录。
- **`validate_port_env` 必须在 `install_plan` 里被调用**(A6a 留下的硬要求),并有"保留名被拒绝"的测试。
- **命令的第一个词必须是该运行时自己的解释器**:Node 只允许 `node`/`npm`/`npx`,Python 只允许 `python`/`python3`/`uv`;其余(`sh`、`./server`、绝对路径)在 `install_plan` 就拒绝——应用清单不能借 `command` 运行任意程序。lockfile 只认 `package-lock.json`(`npm ci`)与 `uv.lock`(`uv sync --frozen`),其余拒绝。
- **端点语义**:进程型应用的 `start` 在监管线程起来后**立即返回**站点地址(`Started{url}` 的含义是"已受理"),观察态依次为 `Preparing`(装依赖)→ `Starting`(起进程/等健康)→ `Running`;`launch_url_if_running` 只在 `Running` 时给地址(已有逻辑,不改)。
- **重启策略沿用 `RestartPolicy::default()`**:放弃 = `Failed{retryable:false}` 且 `desired` 保持 `Running`(`next_action` 对不可重试的失败不再动作,需要人 `stop` 复位后再 `start`)。运行时缺失 = `Failed{retryable:true}` + `RuntimeUnavailable` 事件。
- **静态站点行为完全不变**:既有 manager/dozerd/gateway 测试除 `only_static_web_is_supported_in_phase_one`(改为"容器仍不支持")外一律不改地通过。
- `bytehost-apps` 默认 feature 依赖不变;`scripts/check-bytehost-apps-deps.sh` 必须通过;`cargo clippy -p bytehost-apps -p dozerd --all-targets` 无新警告。
- 日志走 `dozer_core::log`(dozerd 内);`bytehost-apps` 内不新增 `tracing`/`eprintln!`(它不依赖 dozer crate,错误经事件/返回值上报)。

## Review Focus

- **停止与监管线程的竞态**:`stop` 恰好发生在"健康探测成功、线程正要 `ready()`"之间——线程必须在 `ready()` 的锁内看到 `cancel` 并放弃,而不是把已被停止的应用又写回 `Running`/注册上游(Task 4/5 各有测试)。
- **dozerd 被强杀后的孤儿**:`reconcile` 先 `reap_orphan` 再按 `desired` 拉起;端口被孤儿占着时新进程不会拿到同一个端口(每次 `pick_free_port`)。
- **崩溃循环不能紧循环、也不能无限重启**:`GiveUp` 之后线程退出、观察态 `Failed{retryable:false}`,gateway 上游已撤。
- **应用起不来时 `stop` 不能被拖住**:启动期(装依赖/等健康)收到取消,依赖安装进程与应用进程都要被整组收掉,`stop` 在 宽限+1s 内返回。
- **运行时缺失不能让应用卡在 `Starting`**:`resolve` 失败 → `Failed{retryable:true}`、`RuntimeUnavailable` 事件、`start` 返回错误。
- **lockfile 变化要重装依赖**:装好依赖的标记按 lockfile 的 sha256 命名;换了 lockfile 的新版本必须重新装。

## 文件结构

- 新增 `crates/bytehost-apps/src/runtime/resolve.rs` — `RuntimeResolver`/`SystemResolver`(解释器 → 绝对路径 + `PATH` 目录)
- 新增 `crates/bytehost-apps/src/launch.rs` — 进程型 runtime 的清单校验(`check_process_runtime`)与依赖安装命令(`install_argv`)(纯函数)
- 新增 `crates/bytehost-apps/src/supervisor.rs` — 监管线程 `run` 与 `Transitions`
- 修改 `crates/bytehost-apps/src/manager.rs` — `Core` 抽取、进程分支、取消/收尾、`reconcile` 回收孤儿
- 修改 `crates/bytehost-apps/src/registry.rs` — `AppPaths::run_dir`
- 修改 `crates/bytehost-apps/src/lib.rs`、`runtime/mod.rs`
- 修改 `crates/dozerd/src/app_service.rs`(测试)、`CLAUDE.md`(bytehost-apps 行)

---

### Task 1: `RuntimeResolver`——把 `node`/`python` 解析成绝对路径

**Files:** Create `crates/bytehost-apps/src/runtime/resolve.rs`;Modify `crates/bytehost-apps/src/runtime/mod.rs`(`mod resolve; pub use resolve::{...};`)。

**Interfaces:**
- Produces:
  ```rust
  pub struct Resolved { pub program: PathBuf, pub path_dirs: Vec<PathBuf> }
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum ResolveError { NotInstalled(String) }   // 载荷:解释器名
  pub trait RuntimeResolver: Send + Sync { fn resolve(&self, program: &str) -> Result<Resolved, ResolveError>; }
  pub struct SystemResolver { /* 额外搜索目录 */ }
  impl SystemResolver { pub fn new() -> Self; pub fn with_dirs(dirs: Vec<PathBuf>) -> Self; }
  ```
  `resolve("npm")` 的 `path_dirs` 必须含 `npm` 所在目录(npm 是 `#!/usr/bin/env node` 脚本,要在同目录找到 `node`);`SystemResolver::new()` 的搜索顺序:`$PATH` 里的目录,再 `/opt/homebrew/bin`、`/usr/local/bin`、`$HOME/.local/bin`、`$HOME/.cargo/bin`、`$HOME/.volta/bin`(`HOME` 取不到就跳过它们)。

- [ ] **Step 1: 写失败测试**(`resolve.rs` 底部 `mod tests`;用 tempdir 里的假可执行文件,不碰真机环境)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn exe(dir: &std::path::Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn the_first_directory_that_holds_an_executable_wins_and_its_dir_goes_on_the_path() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        exe(b.path(), "node");
        let r = SystemResolver::with_dirs(vec![a.path().into(), b.path().into()])
            .resolve("node")
            .unwrap();
        assert_eq!(r.program, b.path().join("node"));
        assert_eq!(r.path_dirs, vec![b.path().to_path_buf()]);
    }

    #[test]
    fn a_non_executable_file_is_not_a_runtime() {
        let a = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("node"), "x").unwrap(); // 没有执行位
        assert_eq!(
            SystemResolver::with_dirs(vec![a.path().into()]).resolve("node"),
            Err(ResolveError::NotInstalled("node".into()))
        );
    }

    #[test]
    fn only_bare_interpreter_names_are_resolved_never_paths() {
        let a = tempfile::tempdir().unwrap();
        exe(a.path(), "node");
        let r = SystemResolver::with_dirs(vec![a.path().into()]);
        for bad in ["/bin/sh", "../node", "a/b", "", "."] {
            assert!(r.resolve(bad).is_err(), "{bad}");
        }
    }
}
```

- [ ] **Step 2: 确认失败** — `cargo test -p bytehost-apps --all-features runtime::resolve` → 编译失败(类型不存在)。

- [ ] **Step 3: 实现**

```rust
//! 把解释器名(`node`/`npm`/`python3`/`uv`…)解析成绝对路径。dozerd 若由 GUI 拉起,继承来的 `PATH`
//! 往往只有 `/usr/bin:/bin`,所以这里自己带一份常见目录;A6d 装到 bytehost 自己目录里的运行时也经这个 trait 接入。

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub program: PathBuf,
    /// 子进程 `PATH` 的前缀目录(`npm` 要在同目录找到 `node`)。
    pub path_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    NotInstalled(String),
}

pub trait RuntimeResolver: Send + Sync {
    fn resolve(&self, program: &str) -> Result<Resolved, ResolveError>;
}

pub struct SystemResolver {
    dirs: Vec<PathBuf>,
}

impl SystemResolver {
    pub fn new() -> Self {
        let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            dirs.extend([".local/bin", ".cargo/bin", ".volta/bin"].map(|d| home.join(d)));
        }
        Self { dirs }
    }

    pub fn with_dirs(dirs: Vec<PathBuf>) -> Self {
        Self { dirs }
    }
}

impl Default for SystemResolver {
    fn default() -> Self {
        Self::new()
    }
}

fn is_executable_file(p: &Path) -> bool {
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

impl RuntimeResolver for SystemResolver {
    fn resolve(&self, program: &str) -> Result<Resolved, ResolveError> {
        let bare = !program.is_empty()
            && program != "."
            && program != ".."
            && !program.contains('/')
            && !program.contains('\0');
        if bare {
            for dir in &self.dirs {
                let candidate = dir.join(program);
                if is_executable_file(&candidate) {
                    return Ok(Resolved {
                        program: candidate,
                        path_dirs: vec![dir.clone()],
                    });
                }
            }
        }
        Err(ResolveError::NotInstalled(program.to_string()))
    }
}
```

`runtime/mod.rs` 加:`mod resolve;` 与 `pub use resolve::{ResolveError, Resolved, RuntimeResolver, SystemResolver};`。

- [ ] **Step 4: 通过** — 同上命令,3 个测试 PASS。
- [ ] **Step 5: Commit** — `git add crates/bytehost-apps/src/runtime && git commit -m "feat(bytehost-apps): runtime resolver — interpreter name to absolute path with a dependable PATH (A6c task 1)"`(提交信息末尾带仓库约定的 Co-Authored-By 行)。

---

### Task 2: `launch.rs`——进程型清单校验与依赖安装命令(纯函数)

**Files:** Create `crates/bytehost-apps/src/launch.rs`;Modify `lib.rs`(`#[cfg(feature = "server")] pub mod launch;`);Modify `manager.rs` 的 `plan_for`(见 Task 5,本任务只交付函数)。

**Interfaces:**
- Consumes:`process::env::validate_port_env(&str) -> Result<(), PortEnvError>`(`PortEnvError: Display`)、`manifest::Runtime`。
- Produces:
  ```rust
  /// 通过返回 Ok;否则给出人类可读原因(进 `ManagerError::Rejected`)。`package_dir` 用来确认 lockfile 真的在包里。
  pub fn check_process_runtime(runtime: &Runtime, package_dir: &Path) -> Result<(), String>;
  /// 装依赖用的 argv(`npm ci` / `uv sync --frozen`);没有 lockfile 返回 `None`。
  pub fn install_argv(runtime: &Runtime) -> Option<Vec<String>>;
  /// lockfile 的相对路径(进程型且声明了 lockfile 时)。
  pub fn lockfile_of(runtime: &Runtime) -> Option<&str>;
  ```

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ProcessHttp;

    fn node(command: &[&str], lockfile: Option<&str>, port_env: &str) -> Runtime {
        Runtime::Node {
            command: command.iter().map(|s| s.to_string()).collect(),
            lockfile: lockfile.map(String::from),
            node: None,
            http: ProcessHttp { port_env: port_env.into() },
        }
    }

    fn python(command: &[&str], lockfile: Option<&str>) -> Runtime {
        Runtime::Python {
            command: command.iter().map(|s| s.to_string()).collect(),
            lockfile: lockfile.map(String::from),
            python: None,
            http: ProcessHttp { port_env: "PORT".into() },
        }
    }

    fn pkg(files: &[&str]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for f in files {
            std::fs::write(d.path().join(f), "x").unwrap();
        }
        d
    }

    #[test]
    fn reserved_port_variable_names_are_refused() {
        let d = pkg(&[]);
        for bad in ["NODE_OPTIONS", "PATH", "LD_PRELOAD", "DYLD_INSERT_LIBRARIES", "BYTEHOST_HOST"] {
            let err = check_process_runtime(&node(&["node", "a.js"], None, bad), d.path()).unwrap_err();
            assert!(err.contains(bad), "{err}");
        }
        assert!(check_process_runtime(&node(&["node", "a.js"], None, "PORT"), d.path()).is_ok());
    }

    #[test]
    fn the_command_must_start_with_the_runtimes_own_interpreter() {
        let d = pkg(&[]);
        for ok in [vec!["node", "a.js"], vec!["npm", "start"], vec!["npx", "x"]] {
            assert!(check_process_runtime(&node(&ok, None, "PORT"), d.path()).is_ok(), "{ok:?}");
        }
        for bad in [vec!["sh", "-c", "x"], vec!["./server"], vec!["/usr/bin/node", "a.js"], vec!["python3", "a.py"]] {
            assert!(check_process_runtime(&node(&bad, None, "PORT"), d.path()).is_err(), "{bad:?}");
        }
        for ok in [vec!["python", "a.py"], vec!["python3", "-m", "app"], vec!["uv", "run", "a.py"]] {
            assert!(check_process_runtime(&python(&ok, None), d.path()).is_ok(), "{ok:?}");
        }
        assert!(check_process_runtime(&python(&["node", "a.js"], None), d.path()).is_err());
    }

    #[test]
    fn only_known_lockfiles_that_exist_in_the_package_are_accepted() {
        let d = pkg(&["package-lock.json", "uv.lock", "yarn.lock"]);
        assert!(check_process_runtime(&node(&["node", "a.js"], Some("package-lock.json"), "PORT"), d.path()).is_ok());
        assert!(check_process_runtime(&python(&["uv", "run", "a.py"], Some("uv.lock")), d.path()).is_ok());
        // 不认识的格式、跨运行时、不存在
        assert!(check_process_runtime(&node(&["node", "a.js"], Some("yarn.lock"), "PORT"), d.path()).is_err());
        assert!(check_process_runtime(&node(&["node", "a.js"], Some("uv.lock"), "PORT"), d.path()).is_err());
        let empty = pkg(&[]);
        assert!(check_process_runtime(&node(&["node", "a.js"], Some("package-lock.json"), "PORT"), empty.path()).is_err());
    }

    #[test]
    fn install_commands_follow_the_lockfile() {
        assert_eq!(
            install_argv(&node(&["node", "a.js"], Some("package-lock.json"), "PORT")),
            Some(vec!["npm".into(), "ci".into()])
        );
        assert_eq!(
            install_argv(&python(&["uv", "run", "a.py"], Some("uv.lock"))),
            Some(vec!["uv".into(), "sync".into(), "--frozen".into()])
        );
        assert_eq!(install_argv(&python(&["python3", "a.py"], None)), None);
        assert_eq!(install_argv(&Runtime::StaticWeb { source: "web/".into() }), None);
    }
}
```

- [ ] **Step 2: 确认失败**(`cargo test -p bytehost-apps --all-features launch::` 编译失败)。
- [ ] **Step 3: 实现**

```rust
//! 进程型 runtime 的清单校验与依赖安装命令。纯函数:不起进程、不碰网络。

use std::path::Path;

use crate::manifest::Runtime;
use crate::process::env::validate_port_env;

const NODE_PROGRAMS: &[&str] = &["node", "npm", "npx"];
const PYTHON_PROGRAMS: &[&str] = &["python", "python3", "uv"];

pub fn lockfile_of(runtime: &Runtime) -> Option<&str> {
    match runtime {
        Runtime::Node { lockfile, .. } | Runtime::Python { lockfile, .. } => lockfile.as_deref(),
        _ => None,
    }
}

pub fn check_process_runtime(runtime: &Runtime, package_dir: &Path) -> Result<(), String> {
    let (command, lockfile, http, allowed, known_lock) = match runtime {
        Runtime::Node { command, lockfile, http, .. } => {
            (command, lockfile, http, NODE_PROGRAMS, "package-lock.json")
        }
        Runtime::Python { command, lockfile, http, .. } => {
            (command, lockfile, http, PYTHON_PROGRAMS, "uv.lock")
        }
        _ => return Err("不是进程型应用".to_string()),
    };
    validate_port_env(&http.port_env).map_err(|e| format!("runtime.http.port_env 不合法: {e}"))?;
    let program = command.first().map(String::as_str).unwrap_or("");
    if !allowed.contains(&program) {
        return Err(format!(
            "runtime.command 必须以 {} 之一开头(得到 {program:?})",
            allowed.join("/")
        ));
    }
    if let Some(lock) = lockfile {
        if lock != known_lock {
            return Err(format!("不支持的 lockfile {lock:?}(这种应用只认 {known_lock})"));
        }
        if !package_dir.join(lock).is_file() {
            return Err(format!("包里没有声明的 lockfile {lock}"));
        }
    }
    Ok(())
}

pub fn install_argv(runtime: &Runtime) -> Option<Vec<String>> {
    let to = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
    match (runtime, lockfile_of(runtime)) {
        (Runtime::Node { .. }, Some(_)) => Some(to(&["npm", "ci"])),
        (Runtime::Python { .. }, Some(_)) => Some(to(&["uv", "sync", "--frozen"])),
        _ => None,
    }
}
```

- [ ] **Step 4: 通过**。
- [ ] **Step 5: Commit** — `feat(bytehost-apps): process runtime manifest checks and install commands (A6c task 2)`。

---

### Task 3: 重构——`AppManager` 抽出 `Arc<Core>`(行为不变)

**Files:** Modify `crates/bytehost-apps/src/manager.rs`、`crates/bytehost-apps/src/registry.rs`(`AppPaths::run_dir`)。

**Interfaces:**
- Produces:
  ```rust
  // registry.rs
  impl AppPaths { pub fn run_dir(&self, id: &AppId) -> PathBuf /* app_dir/run */ }
  // manager.rs(私有)
  struct Core { registry, gateway, host_version, events, lock, closed, resolver: Arc<dyn RuntimeResolver>, supervisions: Mutex<HashMap<AppId, Supervision>>, #[cfg(test)] prepared }
  pub struct AppManager { core: Arc<Core> }   // impl Deref<Target = Core>
  impl AppManager { pub fn with_resolver(root, host_version, gateway, resolver: Arc<dyn RuntimeResolver>) -> io::Result<Self> }  // new() = with_resolver(.., Arc::new(SystemResolver::new()))
  struct Supervision { cancel: Arc<AtomicBool>, thread: Option<JoinHandle<()>> }
  ```
  `Core` 上保留现有的 `guard()`/`emit()`/`set_observed()`/`load_record()`,另加 `try_guard() -> Option<MutexGuard<'_, ()>>`(`try_lock`,毒化时同样取回内部数据)。

- [ ] **Step 1:** 先跑 `cargo test -p bytehost-apps --all-features manager::` 记下通过数量(基线)。
- [ ] **Step 2: 重构。** 把 `AppManager` 的字段整体移进 `Core`;`AppManager { core: Arc<Core> }`;`impl Deref for AppManager { type Target = Core; fn deref(&self) -> &Core { &self.core } }`;把 `guard`/`events`/`emit`/`launch_url`/`load_record`/`set_observed` 这类只用共享字段的方法放进 `impl Core`(`events()` 与 `launch_url()` 仍是 `AppManager` 的公开方法,经 Deref 直接可用,**不需要**改签名)。`AppManager::new(root, host_version, gateway)` 变成调用 `with_resolver(..., Arc::new(SystemResolver::new()))`。`sweep_staging` 照旧在构造时调用。`supervisions` 先建成空表,本任务不使用。
- [ ] **Step 3:** `cargo test -p bytehost-apps --all-features` 全绿(数量与基线一致,**一个测试都不改**),`cargo clippy -p bytehost-apps --all-features --all-targets` 干净。若有测试因私有字段访问(`rig.manager.registry`)编译失败,它们走 Deref 不应受影响;确有受影响的,只允许改成经 Deref 的等价写法。
- [ ] **Step 4: Commit** — `refactor(bytehost-apps): AppManager shares its state through Arc<Core> (A6c task 3)`。

---

### Task 4: `supervisor.rs`——监管线程(与 manager 解耦,真子进程测试)

**Files:** Create `crates/bytehost-apps/src/supervisor.rs`;Modify `lib.rs`(`#[cfg(all(feature = "server", unix))] pub mod supervisor;`)。

**Interfaces:**
- Consumes:`process::{env::{build_env, EnvSpec}, health::{wait_healthy, Health}, restart::{RestartPolicy, RestartTracker, Decision}, supervise::{ProcessSpec, Running, pick_free_port, record_for, write_record, clear_record}}`。
- Produces:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum Phase { Preparing, Starting }

  /// 监管线程向宿主回报。**每个方法返回 `false` = 这次监管已被取消**,线程立刻收尾,不再回报、不再重启。
  pub trait Transitions: Send + Sync + 'static {
      fn phase(&self, phase: Phase) -> bool;
      /// 应用健康了,在 `127.0.0.1:port` 上提供服务。
      fn ready(&self, port: u16) -> bool;
      /// 进程没了/不健康;`restart_in = Some(d)`:`d` 后重启(观察态回 `Starting`),`None`:放弃(`Failed{retryable:false}`)。
      fn down(&self, reason: String, restart_in: Option<std::time::Duration>) -> bool;
      /// 起不来且不该重试(装依赖失败、拿不到端口):`Failed`。
      fn failed(&self, reason: String, retryable: bool) -> bool;
  }

  pub struct Launch {
      pub app_id: AppId,
      pub argv: Vec<String>,                 // 已把 argv[0] 换成绝对路径
      pub install: Option<Install>,
      pub cwd: PathBuf,
      pub parent_env: Vec<(OsString, OsString)>,   // 已带好重建过的 PATH 与 npm/uv 缓存目录变量
      pub port_env: String,
      pub data_dir: PathBuf,
      pub health_path: String,
      pub startup_budget: Duration,          // 默认 30s(`DEFAULT_STARTUP_BUDGET`)
      pub log_path: PathBuf, pub log_max_bytes: u64, pub log_keep: u32,
      pub run_dir: PathBuf,
      pub policy: RestartPolicy,
      pub grace: Duration,                   // 停止宽限,默认 3s
  }
  pub struct Install { pub argv: Vec<String>, pub marker: PathBuf, pub timeout: Duration }

  pub fn run(launch: Launch, cancel: Arc<AtomicBool>, tr: Arc<dyn Transitions>);
  ```

行为(`run` 的主循环,按此写):

```rust
pub fn run(launch: Launch, cancel: Arc<AtomicBool>, tr: Arc<dyn Transitions>) {
    let cancelled = || cancel.load(Ordering::SeqCst);
    // 1. 依赖(装过的标记在就跳过)
    if let Some(inst) = &launch.install {
        if !inst.marker.exists() {
            if !tr.phase(Phase::Preparing) { return; }
            match install_deps(&launch, inst, &cancelled) {
                Install::Done => {}
                Install::Cancelled => return,
                Install::Failed(why) => { tr.failed(why, true); return; }
            }
        }
    }
    // 2. 起进程 → 健康 → 盯着 → 崩了按策略重启
    let mut tracker = RestartTracker::default();
    loop {
        if cancelled() || !tr.phase(Phase::Starting) { return; }
        let Ok(port) = pick_free_port() else { tr.failed("拿不到空闲端口".into(), true); return; };
        let env = app_env(&launch, port);
        let spawned = Running::spawn(&ProcessSpec { argv: launch.argv.clone(), cwd: launch.cwd.clone(), env,
            log_path: launch.log_path.clone(), log_max_bytes: launch.log_max_bytes, log_keep: launch.log_keep });
        let reason;
        let mut ran = Duration::ZERO;
        match spawned {
            Err(e) => reason = format!("起不来: {e}"),
            Ok(mut running) => {
                let _ = write_record(&launch.run_dir, &record_for(running.pid(), launch.argv.clone()));
                let health = wait_healthy(port, &launch.health_path, launch.startup_budget,
                    Duration::from_millis(100), || !cancelled() && running.try_exit().ok().flatten().is_none());
                if cancelled() { let _ = running.stop(launch.grace); clear_record(&launch.run_dir); return; }
                reason = match health {
                    Health::Healthy => {
                        if !tr.ready(port) { let _ = running.stop(launch.grace); clear_record(&launch.run_dir); return; }
                        // 盯着,直到进程退出或被取消
                        loop {
                            if cancelled() { let _ = running.stop(launch.grace); clear_record(&launch.run_dir); return; }
                            if let Ok(Some(status)) = running.try_exit() { break format!("进程退出: {status}"); }
                            std::thread::sleep(Duration::from_millis(200));
                        }
                    }
                    Health::Unhealthy(why) => { let _ = running.stop(launch.grace); format!("健康检查没通过: {why}") }
                };
                ran = running.ran_for();
                clear_record(&launch.run_dir);
            }
        }
        match tracker.on_exit(&launch.policy, Instant::now(), ran) {
            Decision::RestartAfter(d) => {
                if !tr.down(reason, Some(d)) || !sleep_unless_cancelled(&cancel, d) { return; }
            }
            Decision::GiveUp => { tr.down(format!("{reason}(重启次数过多,已放弃)"), None); return; }
        }
    }
}
```

`app_env` = `build_env(launch.parent_env.clone(), &EnvSpec{ app_id, port, port_env, data_dir })`;`install_deps` 用同一 `Running::spawn`(`argv = inst.argv`、端口变量不需要但 `app_env` 复用 port 0)起依赖安装进程,轮询 `try_exit`(200ms)直到退出/超时(`inst.timeout`,默认 10 分钟)/取消;取消或超时都 `stop(launch.grace)`;成功(退出码 0)写 marker(`std::fs::write(marker, "")`,先 `create_dir_all` 父目录),失败返回 `Failed("安装依赖失败: <status>")`;`sleep_unless_cancelled` 以 50ms 为步长睡,取消时返回 `false`。

- [ ] **Step 1: 写失败测试**(`supervisor.rs` 的 `mod tests`,用 `sh` 作被监管的"应用",用一个假 `Transitions` 记录调用序列)。假 `Transitions`:

```rust
#[derive(Default)]
struct Rec { log: Mutex<Vec<String>>, cancel_on: Mutex<Option<String>>, cancel: Arc<AtomicBool> }
impl Rec {
    fn push(&self, s: String) -> bool {
        let stop_here = self.cancel_on.lock().unwrap().as_deref() == Some(s.split(' ').next().unwrap());
        self.log.lock().unwrap().push(s);
        if stop_here { self.cancel.store(true, Ordering::SeqCst); }
        !self.cancel.load(Ordering::SeqCst)
    }
    fn names(&self) -> Vec<String> { self.log.lock().unwrap().iter().map(|s| s.split(' ').next().unwrap().to_string()).collect() }
}
impl Transitions for Rec {
    fn phase(&self, p: Phase) -> bool { self.push(format!("{p:?}")) }
    fn ready(&self, port: u16) -> bool { self.push(format!("ready {port}")) }
    fn down(&self, why: String, r: Option<Duration>) -> bool { self.push(format!("down {why} {r:?}")) }
    fn failed(&self, why: String, retryable: bool) -> bool { self.push(format!("failed {why} {retryable}")) }
}
```

`launch(dir, script)` 辅助:`argv = ["sh","-c", script]`(`script` 里自己用 `$APP_PORT` 起服务),`port_env = "APP_PORT"`,`parent_env = std::env::vars_os().collect()`,`startup_budget = 5s`,`grace = 1s`,策略 `RestartPolicy { max_restarts: 2, window: 60s, base: 50ms, cap: 100ms, stable_after: 60s }`,日志与 run_dir 放在 tempdir。需要一个会回应 HTTP 的"应用":用 `python3 -m http.server $APP_PORT --bind 127.0.0.1`(找不到 `python3` 的测试开头 `return`,与 A6a e2e 一致)。要写的测试:
  1. `a_healthy_app_is_reported_ready_then_cancelling_stops_it_without_further_reports`:`cancel_on = "ready"`;线程返回后记录里最后一项是 `ready ..`,没有 `down`/`failed`;`run_dir` 里没有遗留 `run.json`;`probe_http(port)` 之后是 `Unhealthy`(进程已被收掉)。
  2. `a_crash_loop_restarts_with_backoff_then_gives_up`:`script = "exit 3"`;期望序列 `Starting, down(Some), Starting, down(Some), Starting, down(None)`(`max_restarts=2` ⇒ 第 3 次退出放弃),最后 `down` 的原因含"已放弃";线程自行返回。
  3. `an_app_that_never_answers_is_stopped_and_counted_as_a_failed_start`:`script = "sleep 30"`、`startup_budget = 600ms`:序列以 `Starting, down` 开头,原因含"健康检查没通过",且 `sleep` 进程已被收掉(用 `pgrep`-free 的做法:脚本把 `$$` 写到 tempdir 文件,测试用 `kill(pid, 0)` 断言不存在)。
  4. `dependencies_install_once_and_a_failed_install_is_reported`:`install = Some(Install{ argv: ["sh","-c","echo x >> installs.txt"], marker, timeout 5s })`、应用 `exit 0`:运行两次 `run`(第二次 marker 已在)后 `installs.txt` 恰好一行;`phase` 序列第一次含 `Preparing`、第二次不含;再用 `argv = ["sh","-c","exit 9"]`(新 marker)断言 `failed ... true` 且**没有** `Starting`。
  5. `cancelling_during_the_install_stops_the_install_process`:`install.argv = ["sh","-c","sleep 30"]`,`cancel_on = "Preparing"`;`run` 在 `grace+1s` 内返回,记录里只有 `Preparing`。
  6. `a_failed_spawn_counts_as_a_failed_start_instead_of_panicking`:`argv = ["/definitely/not/here"]`:序列含 `down`,原因含"起不来"。
- [ ] **Step 2: 确认失败**(编译失败)。
- [ ] **Step 3: 实现** `supervisor.rs`(按上方 `run` 骨架,补全 `install_deps`/`app_env`/`sleep_unless_cancelled`/`Install` 结果枚举;文件头写模块注释说明"返回 false = 已取消"的契约)。
- [ ] **Step 4: 通过** — `cargo test -p bytehost-apps --all-features supervisor::`;再连跑 5 遍确认没有偶发(`for i in 1 2 3 4 5; do cargo test -p bytehost-apps --all-features supervisor:: -- --test-threads=4 || break; done`)。
- [ ] **Step 5: Commit** — `feat(bytehost-apps): supervisor thread — deps, spawn, health, restart backoff, cancellation (A6c task 4)`。

---

### Task 5: manager 接线——`plan_for`/`start`/`stop`/`suspend_all`/`reconcile`

**Files:** Modify `crates/bytehost-apps/src/manager.rs`、`crates/bytehost-apps/src/registry.rs`(`run_dir`,若 Task 3 未做)。

**Interfaces:**
- Consumes:Task 1(`RuntimeResolver`)、Task 2(`check_process_runtime`/`install_argv`/`lockfile_of`)、Task 4(`supervisor::{run, Launch, Install, Transitions, Phase}`)、`Gateway::add_upstream`/`remove_site`、`process::supervise::reap_orphan`。
- Produces:
  - `ManagerError::RuntimeUnavailable(String)`(`kind()` → `AppErrorKind::Unavailable`;文案 `运行时 {name} 不可用:未找到`)。
  - `AppManager` 行为变化(公开签名**不变**):`install_plan` 接受 node/python(容器仍 `UnsupportedRuntime("container")`),`start` 对进程型应用起监管线程。
  - 私有:`Core::start_process_locked`、`Core::cancel_supervision(&AppId) -> Option<JoinHandle<()>>`、`struct AppTransitions { core: Arc<Core>, id: AppId, cancel: Arc<AtomicBool> }`(`impl Transitions`)。

要点(按此实现,代码写在 manager.rs):

1. **`plan_for`**:把 `static_source(&package.manifest)?` 换成 `check_supported(&package.manifest, package_dir)?`:`StaticWeb` → 同原逻辑;`Node|Python` → `check_process_runtime(&runtime, package_dir)` 失败映射为 `ManagerError::BadSource(原因)`(线上类别 `Rejected`);`Container` → `UnsupportedRuntime("container")`。`plan_for` 现在需要 `package_dir`:`install_plan` 传源目录,`install_staged` 传 staging(安装时对 staging 副本重新校验)。
2. **`start_locked`**(在读完 manifest 之后分流):

```rust
let record_dir = self.registry.paths().package_dir(id, &record.current_version);
match &manifest.runtime {
    Runtime::StaticWeb { .. } => { /* 原静态逻辑,不动 */ }
    Runtime::Node { .. } | Runtime::Python { .. } => return self.start_process_locked(&mut record, &manifest, &record_dir),
    Runtime::Container { .. } => return Err(ManagerError::UnsupportedRuntime("container".into())),
}
```

`start_process_locked`:
   - 先 `cancel_supervision(id)` 收掉旧的(已结束的监管线程)。
   - 解析:`argv[0]` 经 `self.resolver.resolve(..)`,装依赖的 `argv[0]`(`npm`/`uv`)也解析;任一失败 → `set_observed(Failed{ reason: "运行时 X 未安装", retryable: true })`、`emit(RuntimeUnavailable{ app, reason: RuntimeReason::NotInstalled{ runtime: X } })`、返回 `ManagerError::RuntimeUnavailable(X)`。
   - 组 `Launch`:`argv` 换绝对路径;`parent_env` = `std::env::vars_os()` 去掉原 `PATH` 后加 `PATH = <path_dirs 去重>:/usr/bin:/bin`、`npm_config_cache=<cache_dir>/npm`、`UV_CACHE_DIR=<cache_dir>/uv`;`cwd = package_dir`;`data_dir`/`log_path = logs_dir/app.log`(1 MiB × keep 3)/`run_dir` 取自 `AppPaths`(先 `create_dir_all`);`health_path = manifest.health.path`;`install` 的 `marker = cache_dir/deps-<sha256(lockfile 内容)>.ok`(`sha256_hex`),lockfile 的内容读自包目录。
   - `record.desired = Running`;`set_observed(Starting)`;新建 `cancel`,`thread = std::thread::Builder::new().name(format!("bytehost-app-{id}")).spawn(move || supervisor::run(launch, cancel, Arc::new(AppTransitions{..})))`;登记进 `supervisions`;返回 `gateway.site_url(id)`。
3. **`AppTransitions`**:每个方法先 `let _g = loop { if cancel { return false } if let Some(g) = core.try_guard() { break g } sleep(5ms) }`,拿到锁后再查一次 `cancel`(取消了返回 `false`),然后 `load_record` 并:
   - `phase(p)`:`set_observed(Preparing|Starting)`;
   - `ready(port)`:`gateway.add_upstream(id, 127.0.0.1:port)`、`set_observed(Running)`、`emit(EndpointChanged{ url: Some(site_url) })`;
   - `down(why, Some(d))`:`gateway.remove_site(id)`、`emit(EndpointChanged{None})`(若之前在服务)、`set_observed(Starting)`;`down(why, None)` 与 `failed(why, retryable)`:同样撤上游,`set_observed(Failed{ reason, retryable })`(`down(None)` 的 `retryable=false`)。
   `load_record` 失败(应用被卸载了)→ 返回 `false`。
4. **`stop_locked`**:先 `if let Some(h) = self.cancel_supervision(id) { let _ = h.join(); }`(`cancel_supervision` 置 `cancel`、取出线程句柄;线程在 `grace+1s` 内退出),再走原逻辑;原逻辑里"把观察态落成 `Stopped`"的匹配从 `Running | Failed` 扩成 `Running | Failed | Preparing | Starting`(进程型应用启动中被停止)。
5. **`suspend_all`**:第一阶段(持锁)对 `supervisions` 里**所有**应用置取消并收集句柄;第二阶段(仍持锁,线程不会等锁所以无死锁)逐个 `join`;之后沿用原逻辑(原逻辑里 `observed == Running` → `Stopped` 扩成 `Running | Preparing | Starting`)。总耗时 ≈ 最长的一个宽限,不是 N×宽限。
6. **`reconcile`**:在遍历记录之前,对每个记录先 `reap_orphan(&paths.run_dir(&id), Duration::from_secs(2))`;`Err(e)` 进 `report.problems`("(孤儿回收) <id>", 原因);`Ok(_)` 不报。
7. **`uninstall`** 走 `stop_locked`,不另改;**`list`** 不改。
8. **修改既有测试** `only_static_web_is_supported_in_phase_one`:改名 `containers_are_still_unsupported`,清单换成 `kind = "container"` + `image = "x@sha256:<64 个 a>"` + `[runtime.http]\ncontainer_port = 80`(参考 manifest.rs 里已有的容器样例),断言 `UnsupportedRuntime("container")`。

- [ ] **Step 1: 写失败测试**(manager.rs `mod tests`,用 `rig_with(resolver)`:`AppManager::with_resolver(.., Arc::new(SystemResolver::with_dirs(<tempdir 里放 python3 的软链>)))` 或直接 `SystemResolver::new()`;下列 e2e 测试开头在没有 `python3` 时 `return`)。辅助 `write_py_app(dir, id, version, server_py)`:`manifest.toml` 的 `[runtime]` 为 `kind = "python"`、`command = ["python3", "server.py"]`、`[runtime.http] port_env = "APP_PORT"`,`server.py` 为:

```python
import os, http.server
http.server.test(HandlerClass=http.server.SimpleHTTPRequestHandler, port=int(os.environ["APP_PORT"]), bind="127.0.0.1")
```

  1. `a_python_app_installs_starts_runs_behind_the_gateway_and_stops`:`install` → `start` 返回地址;轮询 `list()` 直到 `observed == Running`(最多 15s);用现有 `rig.fetch(&id, "/")` 得到 200(请求被反向代理到应用);事件序列含 `StateChanged{Starting}`、`StateChanged{Running}`、`EndpointChanged{Some}`;`stop` 后 `observed == Stopped`、`EndpointChanged{None}`、`rig.fetch` 返回 404。
  2. `plans_for_process_apps_reject_reserved_port_names_and_foreign_commands_and_missing_lockfiles`:三份清单(`port_env = "NODE_OPTIONS"`、`command = ["sh","-c","x"]`、`lockfile = "package-lock.json"` 但包里没有)各自 `install_plan` 返回 `Err(BadSource(..))`,`kind()` 为 `Rejected`;并断言 `install_plan` 在合法清单上 `enforcement` 里没有任何 `Enforced`(进程型全是 `Advisory`)。
  3. `a_missing_runtime_fails_the_start_with_a_retryable_failure_and_an_event`:`SystemResolver::with_dirs(vec![])`:`start` 返回 `Err(RuntimeUnavailable("python3"))`,`list` 里是 `Failed{retryable:true}`,事件含 `RuntimeUnavailable{ NotInstalled{ runtime:"python3" } }`。
  4. `stopping_while_the_app_is_still_starting_does_not_leave_it_running`:`server.py` 先 `time.sleep(3)` 再起服务;`start` 之后立刻 `stop`(`Starting` 态):`stop` 在 5s 内返回,最终 `Stopped`;再等 4s 确认状态仍是 `Stopped`、`rig.fetch` 仍 404(监管线程没有把它写回)。
  5. `a_crashing_app_ends_up_failed_and_not_retryable_and_its_site_is_gone`:`server.py` 为 `raise SystemExit(1)`;把重启策略的退避调小需要一个 `#[cfg(test)]` 可注入点:`Core` 加 `policy: RestartPolicy` 字段(默认 `RestartPolicy::default()`),测试里用 `AppManager::with_resolver_and_policy`(`#[cfg(test)]`)传 `{ max_restarts: 1, base: 50ms, cap: 100ms, .. }`;等到 `Failed{retryable:false}`,`rig.fetch` 404,`stop` 之后回到 `Stopped`。
  6. `suspend_all_stops_every_process_and_reconcile_brings_wanted_ones_back`:装并启动两个应用,等 `Running`,`suspend_all`(用 `Instant` 断言总耗时 < 2×宽限+1s)后两个都是 `Stopped` 且 `desired` 仍是 `Running`、端口不再应答;`reconcile` 后两个又回到 `Running`。
  7. `reconcile_reaps_a_leftover_process_recorded_in_the_run_dir`:手工 `Command::new("sleep").arg("306").process_group(0)` 起一个进程,用 `write_record(run_dir, &record_for(pid, vec!["sleep".into(),"306".into()]))` 记录,调 `reconcile`:该进程被杀(`wait` 得到被信号终止),`run.json` 被清掉。
  8. `changing_the_lockfile_reinstalls_dependencies`:用一个假的 `npm` 可执行脚本(tempdir 里 `#!/bin/sh\necho x >> $BYTEHOST_TEST_LOG`……注意白名单环境不会传这个变量,改成脚本里写死 `>> /tmp-dir-absolute/installs.txt` 的绝对路径,路径在生成脚本时拼进去)+ 假 `node`(起 `python3 -m http.server` 的 shell 包装也行);断言同一 lockfile 重启不再装、换了新版本(lockfile 内容不同)再装一次。(若实现者觉得此测试要搭的假运行时过重,可降级为只对 `deps_marker(lockfile_bytes)` 这个纯函数做单测——**但必须保留"标记名随 lockfile 内容变化"这条断言**。)

- [ ] **Step 2: 确认失败**。
- [ ] **Step 3: 实现**(按上方要点)。
- [ ] **Step 4: 通过** — `cargo test -p bytehost-apps --all-features`(整包,含全部既有测试)+ `cargo clippy -p bytehost-apps --all-features --all-targets` + `scripts/check-bytehost-apps-deps.sh`。e2e 测试连跑 3 遍确认稳定。
- [ ] **Step 5: Commit** — `feat(bytehost-apps): process apps through AppManager — plan checks, supervised start, orphan reaping, two-phase suspend (A6c task 5)`。

---

### Task 6: dozerd 端到端 + 文档

**Files:** Modify `crates/dozerd/src/app_service.rs`(测试)、`CLAUDE.md`(bytehost-apps 行)。

**Interfaces:** 无新增公开接口;`AppService` 的 `handle`/`shutdown` 不变(监管逻辑都在 manager 里)。

- [ ] **Step 1: 写测试**(`app_service.rs` 的 `mod tests`,沿用该文件现有的 `install`/`fetch`/`running_url` 辅助;没有 `python3` 时 `return`):
  1. `a_python_app_runs_through_the_wire_and_dies_with_dozerd`:`AppRequest::Plan`→`Install`→`Start`(`AppReply::Started`);轮询 `AppRequest::List` 直到 `Running`;`LaunchUrl` 后 `fetch` 返回应用页面;`service.shutdown().await` 之后应用的端口不再应答(`TcpStream::connect` 失败),且 `List`(新建同根目录的 service)显示 `desired = Running`、`observed = Stopped`——"应用跟随 dozerd"对进程型同样成立。
  2. `a_dozerd_restart_brings_a_python_app_back`:同一根目录重建 `AppService::start_with`(`reconcile` 在其中),轮询到 `Running`。
  3. `a_missing_runtime_is_reported_as_unavailable_not_internal`:`ProbeRuntimes` 不变;用不存在的解释器无法在 dozerd 层注入解析器——此项在 manager 层(Task 5 测试 3)已覆盖,这里只断言 `ManagerError::RuntimeUnavailable` 经 `blocking()` 映射成 `AppErrorKind::Unavailable`(在现有 `failures_are_classified_not_just_described` 里加一行)。
- [ ] **Step 2: 运行** `cargo test -p dozerd app_service`,再 `cargo test -p bytehost-apps -p dozerd -p dozer-app` 与 `cargo clippy --all-targets`、`scripts/check-log-scope.sh`、`scripts/check-bytehost-apps-deps.sh` 全绿。
- [ ] **Step 3: 文档** — `CLAUDE.md` 里 `bytehost-apps` 一行:把"`process` 模块是进程型应用的监管核心……A6a 只有机制、未接 `AppManager`,`validate_port_env` 在 A6c 接线时必须被 `install_plan` 调用"改为:进程型应用已接入(A6c)——`AppManager` 持 `Arc<Core>`,`supervisor.rs` 是监管线程、`launch.rs` 是清单校验(命令只认运行时自己的解释器、lockfile 只认 `package-lock.json`/`uv.lock`、`validate_port_env` 已在 `install_plan` 里调用)、`RuntimeResolver` 解析解释器路径;**监管线程回写状态必须持锁并核对取消标志、拿锁用尝试循环(否则与 `stop` 持锁 `join` 死锁)**;进程型应用的出站网络强制等级仍是 `Advisory`;运行时安装是 A6d。
- [ ] **Step 4: Commit** — `test(dozerd): python apps end to end through the wire; docs: A6c landed (A6c task 6)`。

---

## 已知局限(写在这里,不是缺陷)

- **依赖会写进包目录**:`npm ci`/`uv sync` 在 `package/<ver>/` 里生成 `node_modules`/`.venv`,"包目录不可变"的约定对进程型应用只管到安装时的摘要核对;升级会新建一个版本目录并重新装依赖。
- **`npm ci` 会执行依赖的安装脚本**(安装计划的 `will_run` 已如实写出);没有沙箱,强制等级 `Advisory`。
- **`manifest.node`/`manifest.python`(版本要求)本片不读**:用解析到的系统版本;版本固定与安装是 A6d。
- **没有日志查看界面**:日志在 `logs/app.log`(轮转,总量有上限);`LogAvailable` 事件与界面入口留待后续。
- **每次重启换一个新端口**:应用若把端口写进持久化数据会失效;`port_env` 是它唯一该信任的来源。
- **健康只在启动期探测**:运行中不做周期性健康检查,只看进程是否退出(进程活着但卡死不会被重启)。
- **GUI 不改**:`Preparing`/`Starting` 已显示"启动中…"、`Failed` 已显示崩溃页(见 `app_host.rs`);启动期间 GUI 靠既有的 `List` 轮询看到 `Running`。

## Self-Review(写计划时已核对)

- **Spec 覆盖**:§4.4 desired/observed 与对账(Task 5 的 reconcile + 退避/放弃)、§6.1 进程跟随 dozerd + 孤儿回收(Task 5.5/5.6、Task 6)、§6.2 先探测系统已有的(Task 1);安装到 bytehost 目录(A6d)与不可用提示页(§6.3,GUI 现有"启动失败"呈现,专用提示页另立)在范围外并已写明。A6a 留下的"`validate_port_env` 必须被调用"由 Task 2/5 与"保留名被拒绝"的测试覆盖。
- **占位符**:无;Task 4 的 `run` 给出完整骨架,辅助函数的行为逐条写明。
- **类型一致**:`Transitions`/`Launch`/`Install`/`Phase` 在 Task 4 定义,Task 5 按同名使用;`RuntimeUnavailable`(`ManagerError`)与 `AppEvent::RuntimeUnavailable` 是两个不同的东西,前者是返回值、后者是事件,计划里分开写了。
