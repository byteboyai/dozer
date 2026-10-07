# bytehost A6a:进程型应用的监管核心 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 应用宿主二期的第一片:把 Node/Python 应用当作**子进程**跑起来的机制——启动(独立进程组、白名单环境)、日志(有上限、会轮转)、健康探测、崩溃后的重启退避、优雅停止(整组)、dozerd 被强杀后的孤儿清理。**只有机制**:不接 `AppManager`、gateway、协议、界面。每一块都能不起 GUI、不连 dozerd 地单测,其中进程相关的用**真的子进程**测。

**Architecture:** `bytehost-apps` 新增 `process` 模块(`server` feature;唯一新增依赖是 `libc`,已在依赖树里,锁文件只多一行):`restart`(纯函数)、`env`(纯函数)、`log`(轮转写入 + 管道泵线程)、`health`(阻塞式 HTTP 探测,仅标准库)、`supervise`(unix:`Running`/孤儿清理)。

**Tech Stack:** Rust、`libc`(进程组信号)、`serde_json`(孤儿记录)、测试用 `sh`/`perl`/`python3`(缺 `perl`/`python3` 的机器上相应测试自动跳过)。

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`(§4.4 生命周期、§6 进程/运行时、A9)。

## A6 的切分(规格没有定义 A6,本计划与用户确认:A6 = 进程型运行时)

| 片 | 内容 | 状态 |
|---|---|---|
| **A6a(本计划)** | 监管核心:启动/停止/健康/重启退避/日志/孤儿清理(机制,不接线) | 可执行 |
| **A6b** | gateway 反向代理:`<id>.localhost` → 应用的回环端口,含 **WebSocket** 与流式应答(SSE)、`Host`/`Origin` 处理、令牌 Cookie 仍由 gateway 校验 | 待写 |
| **A6c** | 接入 `AppManager`/dozerd/GUI:`install_plan` 不再拒绝 node/python(校验 `port_env`/lockfile)、`Preparing`(装依赖)、`Starting`、监管线程把 `Running`/`Failed` 写回注册表、`reconcile` 与孤儿清理、状态事件、面板「启动中」;运行时二进制解析(见下"已知局限") | 待写 |
| **A6d** | Settings 里安装 uv/Python/Node(规格 A9:固定版本 + 校验和 + 显式确认,装到 bytehost 自己的目录);探测与安装 UI | 待写 |

## Global Constraints

- **不继承 dozerd 的环境**:子进程环境是白名单(`PATH HOME USER LOGNAME LANG TMPDIR TZ LC_*`)+ 宿主给的 `BYTEHOST_APP_ID`/`BYTEHOST_DATA_DIR`/`BYTEHOST_HOST` + 端口变量。dozerd 的环境里可能有任何密钥/令牌/`DOZER_*`。
- **端口变量名不能成为后门**:`port_env` 必须是 `[A-Z_][A-Z0-9_]*`,且不得是 `PATH`/`HOME`/`NODE_OPTIONS`/`PYTHONPATH`/`LD_*`/`DYLD_*`/`BYTEHOST_*` 等保留名(否则恶意清单可用 `NODE_OPTIONS=--require /evil.js` 之类在子进程里执行任意代码)。
- **每个应用一个独立进程组**,停止时对整组发信号(应用自己拉起的孙进程一起结束);`SIGTERM` → 宽限 → `SIGKILL`。
- **`stop` 永远不能无限阻塞**:日志泵线程的收尾有上限(逃出进程组又攥着输出管道的孙进程不能卡死 dozerd 的停止);进程组信号失败(组不存在)时直接杀领头进程,不能无限 `wait`。
- **孤儿清理只杀"命令行核对一致"的进程**:pid 被系统复用后绝不能误杀无关进程;对不上只清掉记录。
- 健康判据:能连上并收到非 5xx 的 HTTP 状态行(纯 API 应用根路径 404 也算活着);进程活着 ≠ 就绪。
- 日志有总量上限(`max_bytes × (keep+1)`),应用不能把磁盘写满;子进程不直接写文件,经管道泵。
- `bytehost-apps` 默认 feature 的依赖仍只有 serde 家族;`dozer-hook` 闭包不得多出 `libc`(`scripts/check-bytehost-apps-deps.sh` 必须仍通过)。
- 测试里**不要用"绑 0 再释放"来造一个肯定关闭的端口**(并行测试会抢走刚释放的端口,偶发失败),用回环上基本无人监听的端口 1。

## Review Focus

- `env::build_env` 的白名单:确认没有任何看起来无害但会泄露密钥的变量被放行(`SSH_AUTH_SOCK`、`*_TOKEN`、`*_KEY`、`DOZER_*` 均在测试里被钉成"不放行")。
- `validate_port_env` 的保留名清单是否漏了会被解释器当成代码注入点的变量(`PYTHONSTARTUP`、`NODE_OPTIONS`、`LD_PRELOAD`、`DYLD_INSERT_LIBRARIES` 已在);**注意它目前不被任何调用方使用——A6c 在 `install_plan` 里必须调它,并为此加测试**。
- `supervise::Running::stop` 的三条出口:正常退出、`SIGTERM` 被忽略(宽限后 `SIGKILL`)、整组信号失败(杀领头进程)——各有测试或变异证据。
- `reap_orphan` 比较的是 `ps -ww -o command=` 与 `argv.join(" ")`:含空格的参数会让两边都对不上(宁可不杀),**不会**误杀。
- 重启策略:稳定运行(≥ `stable_after`)清历史;窗口内超过 `max_restarts` 放弃——放弃之后由调用方标 `Failed{retryable:false}`(A6c)。

---

### Task 1: 纯逻辑与 I/O 小件(`restart`、`env`、`log`、`health`)

**Files:**
- Create: `crates/bytehost-apps/src/process/{mod.rs,restart.rs,env.rs,log.rs,health.rs}`
- Modify: `crates/bytehost-apps/Cargo.toml`(`libc` 可选依赖,`server` feature 带上)、`crates/bytehost-apps/src/lib.rs`(`pub mod process;`)、`Cargo.lock`(只多一行 `"libc",`)

**Interfaces:**
- Produces: `restart::{RestartPolicy, RestartTracker, Decision}`;`env::{validate_port_env, PortEnvError, EnvSpec, build_env}`;`log::{RotatingLog, pump}`;`health::{Health, probe_http, wait_healthy}`。

- [ ] **Step 1: 写失败测试。** 每个文件先只放骨架(`todo!()`)和下面各自的 `tests` 模块;`mod.rs` 先不含 `supervise`。
- [ ] **Step 2: RED。** `cargo test -p bytehost-apps --all-features process::` → 失败。
- [ ] **Step 3: 实现。**

`crates/bytehost-apps/Cargo.toml` 与 `lib.rs` 的改动:

```diff
+    "dep:libc",   # 加在 server = [ … ] 列表末尾
+# 进程组信号(`kill(-pgid, SIGTERM)`)与存活探测;已在依赖树里(tokio 等依赖它),不新增 crate。
+libc = { version = "0.2", optional = true }   # 加在 [dependencies] 里 uuid 之后
```

```diff
-//! - `server`:`gateway`、`runtime`、`manager`、`port`(只有 dozerd 打开;隐含 `digest` 与 `manifest-toml`)
+//! - `server`:`gateway`、`runtime`、`manager`、`port`、`process`(只有 dozerd 打开;隐含 `digest` 与 `manifest-toml`)
...
+#[cfg(feature = "server")]
+pub mod process;      # 加在 `pub mod port;` 之后
```

`process/mod.rs`(Task 2 再追加 `supervise` 与 `e2e`;此处先不含):

```rust
//! 进程型应用的**监管核心**(bytehost A6a):把一个 Node/Python 应用当作子进程跑起来、盯着、停掉——
//! 只有机制,**不接** `AppManager`、gateway、协议(那是 A6b/A6c)。拆成互相独立、可单测的小块:
//!
//! - `restart`:崩溃后的重启退避与放弃(纯函数);
//! - `env`:给子进程的**白名单环境**(不继承 dozerd 的密钥/令牌)与端口环境变量名校验(纯函数);
//! - `log`:有大小上限、会轮转的日志文件与"把子进程输出泵进去"的线程;
//! - `health`:阻塞式 HTTP 健康探测(进程活着 ≠ 服务就绪);
//! - `supervise`:启动(独立进程组)、存活检查、优雅停止(SIGTERM→宽限→SIGKILL,整组)与孤儿清理。

pub mod env;
pub mod health;
pub mod log;
pub mod restart;
```

`process/restart.rs`:

```rust
//! 崩溃后的重启策略(纯函数,时间由调用方注入)。
//!
//! 规则:每次退出记一笔;一次**稳定运行**(≥ `stable_after`)就清掉历史(它不是崩溃循环);否则按指数退避
//! (`base * 2^(n-1)`,封顶 `cap`)重启,窗口 `window` 内的退出次数超过 `max_restarts` 就**放弃**——
//! 由调用方把应用标成 `Failed{ retryable: false }`,不在后台无限地重启一个起不来的应用。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartPolicy {
    pub max_restarts: u32,
    pub window: Duration,
    pub base: Duration,
    pub cap: Duration,
    pub stable_after: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            max_restarts: 5,
            window: Duration::from_secs(10 * 60),
            base: Duration::from_secs(1),
            cap: Duration::from_secs(60),
            stable_after: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    RestartAfter(Duration),
    GiveUp,
}

#[derive(Debug, Default)]
pub struct RestartTracker {
    exits: VecDeque<Instant>,
}

impl RestartTracker {
    /// 进程在 `now` 退出,它这次跑了 `ran_for`。
    pub fn on_exit(&mut self, policy: &RestartPolicy, now: Instant, ran_for: Duration) -> Decision {
        if ran_for >= policy.stable_after {
            self.exits.clear();
        }
        while self
            .exits
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) > policy.window)
        {
            self.exits.pop_front();
        }
        self.exits.push_back(now);
        let n = self.exits.len() as u32;
        if n > policy.max_restarts {
            return Decision::GiveUp;
        }
        let factor = 1u32.checked_shl(n - 1).unwrap_or(u32::MAX);
        Decision::RestartAfter(policy.base.saturating_mul(factor).min(policy.cap))
    }

    /// 用户手动启动/停止后清掉历史(重新开始计数)。
    pub fn reset(&mut self) {
        self.exits.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> RestartPolicy {
        RestartPolicy {
            max_restarts: 3,
            window: Duration::from_secs(100),
            base: Duration::from_secs(1),
            cap: Duration::from_secs(5),
            stable_after: Duration::from_secs(30),
        }
    }

    #[test]
    fn crashes_back_off_exponentially_up_to_the_cap_then_give_up() {
        let mut t = RestartTracker::default();
        let t0 = Instant::now();
        let quick = Duration::from_millis(100);
        assert_eq!(
            t.on_exit(&p(), t0, quick),
            Decision::RestartAfter(Duration::from_secs(1))
        );
        assert_eq!(
            t.on_exit(&p(), t0, quick),
            Decision::RestartAfter(Duration::from_secs(2))
        );
        assert_eq!(
            t.on_exit(&p(), t0, quick),
            Decision::RestartAfter(Duration::from_secs(4))
        );
        assert_eq!(
            t.on_exit(&p(), t0, quick),
            Decision::GiveUp,
            "窗口内第 4 次"
        );
    }

    #[test]
    fn the_backoff_is_capped() {
        let mut t = RestartTracker::default();
        let policy = RestartPolicy {
            max_restarts: 20,
            ..p()
        };
        let t0 = Instant::now();
        let mut last = Duration::ZERO;
        for _ in 0..10 {
            if let Decision::RestartAfter(d) = t.on_exit(&policy, t0, Duration::ZERO) {
                last = d;
            }
        }
        assert_eq!(last, policy.cap);
    }

    #[test]
    fn a_stable_run_clears_the_history() {
        let mut t = RestartTracker::default();
        let t0 = Instant::now();
        for _ in 0..3 {
            t.on_exit(&p(), t0, Duration::ZERO);
        }
        assert_eq!(
            t.on_exit(&p(), t0, Duration::from_secs(31)),
            Decision::RestartAfter(Duration::from_secs(1)),
            "跑稳之后的退出重新从第一次算"
        );
    }

    #[test]
    fn old_exits_fall_out_of_the_window() {
        let mut t = RestartTracker::default();
        let t0 = Instant::now();
        for _ in 0..3 {
            t.on_exit(&p(), t0, Duration::ZERO);
        }
        let later = t0 + Duration::from_secs(101);
        assert_eq!(
            t.on_exit(&p(), later, Duration::ZERO),
            Decision::RestartAfter(Duration::from_secs(1))
        );
    }

    #[test]
    fn reset_starts_counting_again() {
        let mut t = RestartTracker::default();
        let t0 = Instant::now();
        for _ in 0..3 {
            t.on_exit(&p(), t0, Duration::ZERO);
        }
        t.reset();
        assert_eq!(
            t.on_exit(&p(), t0, Duration::ZERO),
            Decision::RestartAfter(Duration::from_secs(1))
        );
    }
}
```

`process/env.rs`:

```rust
//! 给子进程的环境(纯函数)。**白名单,不是黑名单**:dozerd 的环境里可能有任何东西(Agent 令牌、云凭据、`DOZER_*`),
//! 第三方/Agent 生成的应用一概不该继承;只放行运行程序所必需的几个系统变量,再加宿主明确给的几个。

use std::ffi::{OsStr, OsString};
use std::path::Path;

use crate::id::AppId;

/// 从父进程环境里放行的变量名(精确匹配)与前缀。
const PASS_EXACT: &[&str] = &["PATH", "HOME", "USER", "LOGNAME", "LANG", "TMPDIR", "TZ"];
const PASS_PREFIX: &[&str] = &["LC_"];

/// 宿主自己设的变量(`port_env` 不得与它们或系统关键变量重名)。
const RESERVED: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "TMPDIR",
    "TZ",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "NODE_OPTIONS",
    "PYTHONPATH",
    "PYTHONSTARTUP",
];

#[derive(Debug, PartialEq, Eq)]
pub enum PortEnvError {
    Empty,
    BadName(String),
    Reserved(String),
}

impl std::fmt::Display for PortEnvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "port_env 不能为空"),
            Self::BadName(n) => write!(
                f,
                "port_env {n:?} 不是合法的环境变量名(只允许 A-Z、0-9、_,且不以数字开头)"
            ),
            Self::Reserved(n) => write!(f, "port_env {n:?} 是保留变量名"),
        }
    }
}

impl std::error::Error for PortEnvError {}

/// 校验 manifest 里的 `http.port_env`:大写字母/数字/下划线、不以数字开头,且不是系统关键变量或 `BYTEHOST_*`。
/// (否则一个恶意清单可以让端口变量覆盖 `PATH`/`NODE_OPTIONS`/`LD_PRELOAD`,在子进程里执行任意代码。)
pub fn validate_port_env(name: &str) -> Result<(), PortEnvError> {
    if name.is_empty() {
        return Err(PortEnvError::Empty);
    }
    let ok = name
        .bytes()
        .enumerate()
        .all(|(i, b)| b.is_ascii_uppercase() || b == b'_' || (i > 0 && b.is_ascii_digit()));
    if !ok {
        return Err(PortEnvError::BadName(name.to_owned()));
    }
    if RESERVED.contains(&name)
        || name.starts_with("BYTEHOST_")
        || name.starts_with("DYLD_")
        || name.starts_with("LD_")
    {
        return Err(PortEnvError::Reserved(name.to_owned()));
    }
    Ok(())
}

pub struct EnvSpec<'a> {
    pub app_id: &'a AppId,
    pub port: u16,
    pub port_env: &'a str,
    /// 应用自己的 `data/` 目录(绝对路径)。
    pub data_dir: &'a Path,
}

/// 组装子进程环境:父环境里的白名单变量 + 宿主给的 `BYTEHOST_APP_ID`/`BYTEHOST_DATA_DIR`/`BYTEHOST_HOST` + 端口变量。
/// `port_env` 必须已通过 [`validate_port_env`](这里不再检查,避免两处各写一份)。
pub fn build_env<I>(parent: I, spec: &EnvSpec<'_>) -> Vec<(OsString, OsString)>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let mut out: Vec<(OsString, OsString)> = parent
        .into_iter()
        .filter(|(k, _)| {
            let k = k.to_string_lossy();
            PASS_EXACT.contains(&k.as_ref()) || PASS_PREFIX.iter().any(|p| k.starts_with(p))
        })
        .collect();
    let mut set = |k: &str, v: &OsStr| out.push((OsString::from(k), v.to_owned()));
    set("BYTEHOST_APP_ID", OsStr::new(spec.app_id.as_str()));
    set("BYTEHOST_DATA_DIR", spec.data_dir.as_os_str());
    set("BYTEHOST_HOST", OsStr::new("127.0.0.1"));
    set(spec.port_env, OsStr::new(&spec.port.to_string()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn os(s: &str) -> OsString {
        OsString::from(s)
    }

    #[test]
    fn only_allow_listed_parent_variables_reach_the_child() {
        let parent = [
            ("PATH", "/usr/bin"),
            ("HOME", "/Users/u"),
            ("LC_ALL", "en_US.UTF-8"),
            ("DOZER_SOCKET", "/x"),
            ("ANTHROPIC_API_KEY", "sk-secret"),
            ("AWS_SECRET_ACCESS_KEY", "s"),
            ("SSH_AUTH_SOCK", "/tmp/agent"),
            ("GITHUB_TOKEN", "t"),
        ]
        .map(|(k, v)| (os(k), os(v)));
        let id = AppId::new("demo").unwrap();
        let data = PathBuf::from("/apps/demo/data");
        let env = build_env(
            parent,
            &EnvSpec {
                app_id: &id,
                port: 24001,
                port_env: "PORT",
                data_dir: &data,
            },
        );
        let keys: Vec<String> = env
            .iter()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        for kept in [
            "PATH",
            "HOME",
            "LC_ALL",
            "BYTEHOST_APP_ID",
            "BYTEHOST_DATA_DIR",
            "BYTEHOST_HOST",
            "PORT",
        ] {
            assert!(keys.contains(&kept.to_string()), "{kept}: {keys:?}");
        }
        for dropped in [
            "DOZER_SOCKET",
            "ANTHROPIC_API_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "SSH_AUTH_SOCK",
            "GITHUB_TOKEN",
        ] {
            assert!(!keys.contains(&dropped.to_string()), "{dropped} 不应继承");
        }
    }

    #[test]
    fn the_host_variables_carry_the_right_values_and_win_over_the_parent() {
        let id = AppId::new("demo").unwrap();
        let data = PathBuf::from("/apps/demo/data");
        // 父环境里就有同名变量时,宿主给的排在后面(后设置的覆盖先设置的)。
        let env = build_env(
            [(os("PATH"), os("/bin"))],
            &EnvSpec {
                app_id: &id,
                port: 31337,
                port_env: "APP_PORT",
                data_dir: &data,
            },
        );
        let get = |k: &str| {
            env.iter()
                .rev()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.to_string_lossy().into_owned())
        };
        assert_eq!(get("APP_PORT").as_deref(), Some("31337"));
        assert_eq!(get("BYTEHOST_APP_ID").as_deref(), Some("demo"));
        assert_eq!(get("BYTEHOST_DATA_DIR").as_deref(), Some("/apps/demo/data"));
        assert_eq!(get("BYTEHOST_HOST").as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn port_env_names_are_validated() {
        for ok in ["PORT", "APP_PORT", "HTTP_PORT_1", "_X"] {
            assert_eq!(validate_port_env(ok), Ok(()), "{ok}");
        }
        assert_eq!(validate_port_env(""), Err(PortEnvError::Empty));
        for bad in ["port", "1PORT", "A-B", "A B", "PORT=1", "P\u{e9}RT", "A.B"] {
            assert!(
                matches!(validate_port_env(bad), Err(PortEnvError::BadName(_))),
                "{bad}"
            );
        }
    }

    /// 端口变量名不能成为覆盖关键变量的后门(`NODE_OPTIONS=--require /evil.js` 就是任意代码执行)。
    #[test]
    fn port_env_cannot_shadow_security_critical_variables() {
        for reserved in [
            "PATH",
            "HOME",
            "NODE_OPTIONS",
            "PYTHONPATH",
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "BYTEHOST_APP_ID",
            "BYTEHOST_X",
            "LD_ANYTHING",
            "DYLD_ANYTHING",
        ] {
            assert!(
                matches!(validate_port_env(reserved), Err(PortEnvError::Reserved(_))),
                "{reserved}"
            );
        }
    }
}
```

`process/log.rs`:

```rust
//! 有大小上限、会轮转的应用日志。子进程的 stdout/stderr 经管道泵进 [`RotatingLog`](不让子进程直接写文件:
//! 一个跑了几个月的应用不能把磁盘写满)。`path` → `path.1` → … → `path.<keep>`,最旧的被丢弃。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

pub struct RotatingLog {
    path: PathBuf,
    max_bytes: u64,
    keep: u32,
    file: File,
    written: u64,
}

impl RotatingLog {
    pub fn open(path: &Path, max_bytes: u64, keep: u32) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let written = file.metadata()?.len();
        Ok(Self {
            path: path.to_path_buf(),
            max_bytes: max_bytes.max(1),
            keep: keep.max(1),
            file,
            written,
        })
    }

    fn numbered(&self, n: u32) -> PathBuf {
        let mut os = self.path.clone().into_os_string();
        os.push(format!(".{n}"));
        PathBuf::from(os)
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        for n in (1..self.keep).rev() {
            let from = self.numbered(n);
            if from.exists() {
                fs::rename(&from, self.numbered(n + 1))?;
            }
        }
        fs::rename(&self.path, self.numbered(1))?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.written = 0;
        Ok(())
    }
}

impl Write for RotatingLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.written > 0 && self.written + buf.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        let n = self.file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// 把 `reader`(子进程的管道)一直读到 EOF,写进 `log`。线程在管道关闭(子进程退出)时结束。
pub fn pump<R: Read + Send + 'static>(mut reader: R, mut log: RotatingLog) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if log.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = log.flush();
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_rotate_at_the_size_limit_and_keep_a_bounded_history() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("app.log");
        let mut log = RotatingLog::open(&p, 10, 2).unwrap();
        for chunk in ["aaaaaa", "bbbbbb", "cccccc", "dddddd"] {
            log.write_all(chunk.as_bytes()).unwrap();
        }
        log.flush().unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "dddddd");
        assert_eq!(
            fs::read_to_string(dir.path().join("app.log.1")).unwrap(),
            "cccccc"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("app.log.2")).unwrap(),
            "bbbbbb"
        );
        assert!(!dir.path().join("app.log.3").exists(), "只留 keep=2 份历史");
    }

    #[test]
    fn a_single_oversized_write_is_not_split_or_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("app.log");
        let mut log = RotatingLog::open(&p, 4, 1).unwrap();
        log.write_all(b"0123456789").unwrap();
        log.flush().unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "0123456789");
    }

    #[test]
    fn reopening_appends_and_counts_what_is_already_there() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("app.log");
        {
            let mut log = RotatingLog::open(&p, 10, 1).unwrap();
            log.write_all(b"12345678").unwrap();
        }
        let mut log = RotatingLog::open(&p, 10, 1).unwrap();
        log.write_all(b"abc").unwrap();
        log.flush().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("app.log.1")).unwrap(),
            "12345678"
        );
        assert_eq!(fs::read_to_string(&p).unwrap(), "abc");
    }

    #[test]
    fn the_pump_copies_a_pipe_into_the_log_until_eof() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("app.log");
        let log = RotatingLog::open(&p, 1024, 1).unwrap();
        let data: &'static [u8] = b"line1\nline2\n";
        pump(data, log).join().unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "line1\nline2\n");
    }
}
```

`process/health.rs`:

```rust
//! 阻塞式 HTTP 健康探测:进程活着不等于服务已经在监听/能应答。只用标准库(监管线程里跑,不引入 async)。
//! 判据:能建立连接并收到一个 HTTP 状态行,且状态码不是 5xx(根路径返回 404 的纯 API 应用也算活着)。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    Healthy,
    Unhealthy(String),
}

/// 解析状态行 `HTTP/1.x <code> ...`;不是 HTTP 返回 `None`。
fn status_code(head: &[u8]) -> Option<u16> {
    let line = head.split(|b| *b == b'\n').next()?;
    let line = std::str::from_utf8(line).ok()?.trim_end();
    let rest = line.strip_prefix("HTTP/1.")?;
    let mut it = rest.splitn(3, ' ');
    it.next()?;
    it.next()?.parse().ok()
}

/// 探测 `127.0.0.1:<port><path>` 一次。
pub fn probe_http(port: u16, path: &str, timeout: Duration) -> Health {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = match TcpStream::connect_timeout(&addr, timeout) {
        Ok(s) => s,
        Err(e) => return Health::Unhealthy(format!("连接失败: {e}")),
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nUser-Agent: bytehost-health\r\n\r\n"
    );
    if let Err(e) = stream.write_all(req.as_bytes()) {
        return Health::Unhealthy(format!("发送失败: {e}"));
    }
    let mut buf = [0u8; 256];
    let n = match stream.read(&mut buf) {
        Ok(0) => return Health::Unhealthy("对端直接关闭了连接".into()),
        Ok(n) => n,
        Err(e) => return Health::Unhealthy(format!("读取失败: {e}")),
    };
    match status_code(&buf[..n]) {
        Some(code) if code < 500 => Health::Healthy,
        Some(code) => Health::Unhealthy(format!("状态码 {code}")),
        None => Health::Unhealthy("应答不是 HTTP".into()),
    }
}

/// 轮询到健康、超时、或进程已经死了(`alive()` 为假)为止。返回最后一次的结果。
pub fn wait_healthy(
    port: u16,
    path: &str,
    total: Duration,
    interval: Duration,
    mut alive: impl FnMut() -> bool,
) -> Health {
    let deadline = Instant::now() + total;
    let per_try = interval
        .max(Duration::from_millis(200))
        .min(Duration::from_secs(2));
    loop {
        if !alive() {
            return Health::Unhealthy("进程已经退出".into());
        }
        let h = probe_http(port, path, per_try);
        if h == Health::Healthy || Instant::now() + interval >= deadline {
            return h;
        }
        std::thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// 一个肯定没人监听的回环端口。**不能用"绑 0 再释放"**:测试并行时,刚释放的端口可能被别的测试立刻重新绑走,
    /// 探测就会连上无关的服务(偶发失败)。端口 1(tcpmux)在回环上基本不会有人监听,连接会被拒绝。
    const REFUSED_PORT: u16 = 1;

    /// 起一个只应答一次的假服务,返回端口。
    fn serve_once(reply: &'static [u8]) -> u16 {
        let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf);
                let _ = s.write_all(reply);
            }
        });
        port
    }

    #[test]
    fn the_status_line_is_parsed_strictly() {
        assert_eq!(status_code(b"HTTP/1.1 200 OK\r\n\r\n"), Some(200));
        assert_eq!(status_code(b"HTTP/1.0 404 Not Found\r\n"), Some(404));
        assert_eq!(status_code(b"HTTP/1.1 503\r\n"), Some(503));
        for bad in [
            &b"SSH-2.0-OpenSSH\r\n"[..],
            b"HTTP/2 200\r\n",
            b"",
            b"HTTP/1.1 abc\r\n",
            b"HTTP/1.1\r\n",
        ] {
            assert_eq!(status_code(bad), None, "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn a_2xx_3xx_or_4xx_answer_means_alive_and_5xx_does_not() {
        let t = Duration::from_secs(2);
        assert_eq!(
            probe_http(serve_once(b"HTTP/1.1 200 OK\r\n\r\n"), "/", t),
            Health::Healthy
        );
        assert_eq!(
            probe_http(serve_once(b"HTTP/1.1 404 Not Found\r\n\r\n"), "/", t),
            Health::Healthy
        );
        assert_eq!(
            probe_http(serve_once(b"HTTP/1.1 302 Found\r\n\r\n"), "/x", t),
            Health::Healthy
        );
        assert!(
            matches!(probe_http(serve_once(b"HTTP/1.1 503 Busy\r\n\r\n"), "/", t), Health::Unhealthy(m) if m.contains("503"))
        );
    }

    #[test]
    fn non_http_closed_and_silent_servers_are_unhealthy() {
        let t = Duration::from_millis(500);
        assert!(matches!(
            probe_http(serve_once(b"garbage\r\n"), "/", t),
            Health::Unhealthy(_)
        ));
        assert!(matches!(
            probe_http(serve_once(b""), "/", t),
            Health::Unhealthy(_)
        ));
        // 端口上没有任何人在听
        let closed = REFUSED_PORT;
        assert!(
            matches!(probe_http(closed, "/", t), Health::Unhealthy(m) if m.contains("连接失败"))
        );
    }

    #[test]
    fn waiting_stops_early_when_the_process_died_and_gives_up_at_the_deadline() {
        let closed = REFUSED_PORT;
        let started = Instant::now();
        let h = wait_healthy(
            closed,
            "/",
            Duration::from_secs(5),
            Duration::from_millis(50),
            || false,
        );
        assert_eq!(h, Health::Unhealthy("进程已经退出".into()));
        assert!(started.elapsed() < Duration::from_secs(1));
        let started = Instant::now();
        let h = wait_healthy(
            closed,
            "/",
            Duration::from_millis(400),
            Duration::from_millis(50),
            || true,
        );
        assert!(matches!(h, Health::Unhealthy(_)));
        assert!(started.elapsed() < Duration::from_secs(3), "超时就放弃");
    }

    #[test]
    fn waiting_succeeds_once_the_service_comes_up_late() {
        let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l); // 先关着
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            // 端口刚被释放,别的测试可能恰好占着:短暂重试绑定
            let bound = (0..40).find_map(|_| {
                TcpListener::bind(("127.0.0.1", port)).ok().or_else(|| {
                    std::thread::sleep(Duration::from_millis(50));
                    None
                })
            });
            if let Some(l) = bound {
                // 应答几次
                for _ in 0..20 {
                    if let Ok((mut s, _)) = l.accept() {
                        let mut buf = [0u8; 256];
                        let _ = s.read(&mut buf);
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
                    }
                }
            }
        });
        let h = wait_healthy(
            port,
            "/",
            Duration::from_secs(5),
            Duration::from_millis(100),
            || true,
        );
        assert_eq!(h, Health::Healthy);
    }
}
```

- [ ] **Step 4: GREEN。** `cargo fmt -p bytehost-apps && cargo test -p bytehost-apps --all-features process::` → 全过(`restart` 5、`env` 4、`log` 4、`health` 5 = 18 个)。
- [ ] **Step 5: 变异检查(**串行**做,每条之后从快照恢复;下面"变异脚本"附录里 `restart`/`env`/`log`/`health` 的 6 条必须都让对应测试 FAILED)。**
- [ ] **Step 6: 稳定性。** 并行跑 30 遍 `cargo test -p bytehost-apps --all-features process::health`(或整个 `process::`)全部通过(这一步专门防"端口被并行测试抢走"的偶发失败)。
- [ ] **Step 7: Commit。** `git add crates/bytehost-apps Cargo.lock && git commit -m "feat(bytehost-apps): process helpers — restart policy, env allow-list, rotating log, health probe (A6a task 1)"`(提交前确认 `Cargo.lock` 只多了 `"libc",` 一行,不带 `.cargo` patch 引起的 `source` 变动)。

### Task 2: 进程监管(`supervise`)+ 端到端

**Files:** Create `crates/bytehost-apps/src/process/supervise.rs`;Modify `process/mod.rs`(加 `#[cfg(unix)] pub mod supervise;` 与 `e2e` 测试模块)

**Interfaces:**
- Consumes: Task 1 的 `log::{RotatingLog, pump}`、`env::build_env`、`health::{wait_healthy, probe_http}`。
- Produces: `supervise::{ProcessSpec, Running, pick_free_port, RunRecord, write_record, clear_record, Reaped, reap_orphan}`;`Running::{spawn, pid, ran_for, try_exit, stop}`。

- [ ] **Step 1: 写失败测试。** `supervise.rs` 的 `tests` 模块(12 个)与 `mod.rs` 的 `e2e` 先写,`Running::spawn` 先 `todo!()`。
- [ ] **Step 2: RED。** `cargo test -p bytehost-apps --all-features process::supervise` → 失败。
- [ ] **Step 3: 实现。**

`process/supervise.rs`:

```rust
//! 子进程的启动、存活检查、优雅停止与孤儿清理(unix)。
//!
//! - 每个应用一个**独立进程组**(`process_group(0)`):停止时对整组发信号,应用自己拉起的孙进程(`npm` → `node`、
//!   `uv run` → `python`)一并结束,不留孤儿;
//! - 停止 = `SIGTERM` → 宽限期 → `SIGKILL`(整组);
//! - dozerd 若被强杀,子进程会变成孤儿并继续占着端口:启动时用 `<run>/process.json` 里记下的 pid+命令行核对后清理
//!   (**核对命令行**,避免 pid 被系统复用后误杀无关进程)。

use std::ffi::OsString;
use std::io;
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::log::{RotatingLog, pump};

/// `stop` 等日志泵线程收尾的最长时间。
const PUMP_JOIN_TIMEOUT: Duration = Duration::from_secs(1);

pub struct ProcessSpec {
    /// `argv[0]` 是程序(按子进程环境里的 `PATH` 查找),其余是参数。
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    /// **完整**环境(不继承父进程),见 [`super::env::build_env`]。
    pub env: Vec<(OsString, OsString)>,
    pub log_path: PathBuf,
    pub log_max_bytes: u64,
    pub log_keep: u32,
}

pub struct Running {
    child: Child,
    pgid: i32,
    started: Instant,
    pumps: Vec<JoinHandle<()>>,
}

fn is_gone(e: &io::Error) -> bool {
    e.raw_os_error() == Some(libc::ESRCH)
}

fn signal_group(pgid: i32, sig: i32) -> io::Result<()> {
    // SAFETY: `kill` 只是发信号;pgid 来自我们自己 spawn 的子进程(`process_group(0)` 使它等于子进程 pid)。
    let r = unsafe { libc::kill(-pgid, sig) };
    if r == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

impl Running {
    pub fn spawn(spec: &ProcessSpec) -> io::Result<Self> {
        let (program, args) = spec
            .argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "argv 为空"))?;
        let mut child = Command::new(program)
            .args(args)
            .current_dir(&spec.cwd)
            .env_clear()
            .envs(spec.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()?;
        let pgid = child.id() as i32;
        let mut pumps = Vec::new();
        for reader in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let log = RotatingLog::open(&spec.log_path, spec.log_max_bytes, spec.log_keep)?;
            pumps.push(pump(reader, log));
        }
        Ok(Self {
            child,
            pgid,
            started: Instant::now(),
            pumps,
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn ran_for(&self) -> Duration {
        self.started.elapsed()
    }

    /// 进程已经退出就返回它的状态(不阻塞)。
    pub fn try_exit(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// 优雅停止整个进程组:`SIGTERM`,最多等 `grace`,还没退就 `SIGKILL`。返回领头进程的退出状态。
    pub fn stop(&mut self, grace: Duration) -> io::Result<ExitStatus> {
        if let Some(status) = self.child.try_wait()? {
            // 领头进程已退,但它的孙进程可能还在:仍对整组补一刀。
            let _ = signal_group(self.pgid, libc::SIGKILL);
            self.join_pumps();
            return Ok(status);
        }
        match signal_group(self.pgid, libc::SIGTERM) {
            Ok(()) => {}
            Err(e) if is_gone(&e) => {}
            Err(e) => return Err(e),
        }
        let deadline = Instant::now() + grace;
        let status = loop {
            if let Some(s) = self.child.try_wait()? {
                break s;
            }
            if Instant::now() >= deadline {
                match signal_group(self.pgid, libc::SIGKILL) {
                    Ok(()) => {}
                    // 组已经不存在而领头进程还活着(进程组没建成):直接杀领头进程,绝不能无限 `wait`。
                    Err(e) if is_gone(&e) => {
                        let _ = self.child.kill();
                    }
                    Err(e) => return Err(e),
                }
                break self.child.wait()?;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // 领头进程退了,整组里残留的(忽略 SIGTERM 的孙进程)也收掉。
        let _ = signal_group(self.pgid, libc::SIGKILL);
        self.join_pumps();
        Ok(status)
    }

    /// 等日志泵线程收尾——但**有上限**:领头进程退了、整组也杀了,正常情况下管道很快就关;可万一有个逃出了进程组的
    /// 孙进程还攥着管道,`join` 会一直阻塞到它退出(几小时甚至永远),把 `stop` 卡死。超时就放手(线程会在管道最终关闭时自己结束)。
    fn join_pumps(&mut self) {
        let deadline = Instant::now() + PUMP_JOIN_TIMEOUT;
        for h in self.pumps.drain(..) {
            while !h.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if h.is_finished() {
                let _ = h.join();
            }
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // 被丢掉时不留子进程(例如 manager 出错路径上忘了 stop)。
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = signal_group(self.pgid, libc::SIGKILL);
            let _ = self.child.wait();
        }
    }
}

/// 在回环上拿一个当前空闲的端口(绑 0 再释放;释放到子进程绑定之间有极小的竞争窗口,调用方遇到
/// "子进程起不来/端口被占"按普通启动失败处理即可)。
pub fn pick_free_port() -> io::Result<u16> {
    Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

// ---------------------------------------------------------------------------------------------
// 孤儿清理
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub pid: u32,
    pub argv: Vec<String>,
}

fn record_path(run_dir: &Path) -> PathBuf {
    run_dir.join("process.json")
}

pub fn write_record(run_dir: &Path, record: &RunRecord) -> io::Result<()> {
    std::fs::create_dir_all(run_dir)?;
    let tmp = run_dir.join("process.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(record).map_err(io::Error::other)?)?;
    std::fs::rename(&tmp, record_path(run_dir))
}

pub fn clear_record(run_dir: &Path) {
    let _ = std::fs::remove_file(record_path(run_dir));
}

#[derive(Debug, PartialEq, Eq)]
pub enum Reaped {
    /// 没有遗留记录。
    Nothing,
    /// 记录里的进程已经不在了(或 pid 被无关进程复用,命令行对不上):只清掉记录,**不动**那个进程。
    Stale,
    /// 找到并结束了遗留的进程组。
    Killed(u32),
}

/// 读进程 `pid` 的完整命令行(`ps -ww -o command= -p`);进程不存在返回 `None`。
fn command_line(pid: u32) -> Option<String> {
    let out = Command::new("ps")
        .args(["-ww", "-o", "command=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!s.is_empty()).then_some(s)
}

/// 命令行是否就是我们当初启动的那条(`ps` 把 argv 用空格拼起来,所以按拼接后的字符串比较)。
fn command_matches(actual: &str, argv: &[String]) -> bool {
    actual == argv.join(" ")
}

/// dozerd 启动时对每个应用调用:有遗留记录且核对一致就结束整个进程组。
pub fn reap_orphan(run_dir: &Path, grace: Duration) -> io::Result<Reaped> {
    let path = record_path(run_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Reaped::Nothing),
        Err(e) => return Err(e),
    };
    let record: RunRecord = match serde_json::from_str(&text) {
        Ok(r) => r,
        Err(_) => {
            clear_record(run_dir);
            return Ok(Reaped::Stale);
        }
    };
    let alive = command_line(record.pid).filter(|c| command_matches(c, &record.argv));
    let Some(_) = alive else {
        clear_record(run_dir);
        return Ok(Reaped::Stale);
    };
    let pgid = record.pid as i32;
    let _ = signal_group(pgid, libc::SIGTERM);
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline && command_line(record.pid).is_some() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = signal_group(pgid, libc::SIGKILL);
    clear_record(run_dir);
    Ok(Reaped::Killed(record.pid))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(dir: &Path, script: &str) -> ProcessSpec {
        ProcessSpec {
            argv: vec!["sh".into(), "-c".into(), script.into()],
            cwd: dir.to_path_buf(),
            env: vec![(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))],
            log_path: dir.join("logs/app.log"),
            log_max_bytes: 1 << 20,
            log_keep: 2,
        }
    }

    fn alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < end, "超时: {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn output_goes_to_the_log_and_the_exit_status_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Running::spawn(&spec(
            dir.path(),
            "echo hello-out; echo hello-err 1>&2; exit 3",
        ))
        .unwrap();
        wait_until("退出", || r.try_exit().unwrap().is_some());
        let status = r.stop(Duration::from_secs(1)).unwrap();
        assert_eq!(status.code(), Some(3));
        let log = std::fs::read_to_string(dir.path().join("logs/app.log")).unwrap();
        assert!(
            log.contains("hello-out") && log.contains("hello-err"),
            "{log}"
        );
    }

    #[test]
    fn the_child_gets_only_the_given_environment() {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY(测试): 只在本进程环境里加一个变量,随后用 env_clear 验证它没传给子进程。
        unsafe { std::env::set_var("BYTEHOST_TEST_SECRET", "leak") };
        let mut s = spec(dir.path(), "env");
        s.env
            .push((OsString::from("ONLY_THIS"), OsString::from("yes")));
        let mut r = Running::spawn(&s).unwrap();
        wait_until("退出", || r.try_exit().unwrap().is_some());
        r.stop(Duration::from_secs(1)).unwrap();
        let log = std::fs::read_to_string(dir.path().join("logs/app.log")).unwrap();
        assert!(log.contains("ONLY_THIS=yes"), "{log}");
        assert!(
            !log.contains("BYTEHOST_TEST_SECRET"),
            "不得继承父进程环境: {log}"
        );
    }

    #[test]
    fn stop_terminates_the_whole_process_group_including_grandchildren() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("grandchild.pid");
        let script = format!("sleep 300 & echo $! > {}; wait", pidfile.display());
        let mut r = Running::spawn(&spec(dir.path(), &script)).unwrap();
        wait_until("孙进程 pid 写出", || {
            pidfile.exists() && !std::fs::read_to_string(&pidfile).unwrap().trim().is_empty()
        });
        let grandchild: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(alive(grandchild));
        r.stop(Duration::from_secs(2)).unwrap();
        wait_until("孙进程结束", || !alive(grandchild));
    }

    /// 逃出进程组(自己 `setsid`)又攥着输出管道的孙进程:`stop` 不能被它卡住(否则 dozerd 的停止/退出会挂死)。
    #[test]
    fn stop_does_not_hang_on_a_process_that_escaped_the_group_and_holds_the_pipe() {
        if Command::new("perl").arg("-v").output().is_err() {
            eprintln!("跳过:没有 perl");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("escapee.pid");
        // perl 先 setsid 再 exec sleep(新会话/新进程组,但继承了 stdout/stderr 管道)
        let script = format!(
            "perl -MPOSIX -e 'setsid(); open(F, \">{}\"); print F $$; close F; exec \"sleep\", \"20\"' & wait",
            pidfile.display()
        );
        let mut r = Running::spawn(&spec(dir.path(), &script)).unwrap();
        wait_until("逃逸进程写出 pid", || {
            std::fs::read_to_string(&pidfile)
                .map(|t| !t.trim().is_empty())
                .unwrap_or(false)
        });
        let escapee: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let started = Instant::now();
        r.stop(Duration::from_secs(1)).unwrap();
        let took = started.elapsed();
        unsafe { libc::kill(escapee, libc::SIGKILL) };
        assert!(
            took < Duration::from_secs(5),
            "stop 被攥着管道的进程卡住了 {took:?}"
        );
    }

    #[test]
    fn a_child_that_ignores_sigterm_is_killed_after_the_grace_period() {
        let dir = tempfile::tempdir().unwrap();
        let ready = dir.path().join("ready");
        let script = format!(
            "trap '' TERM; touch {}; while :; do sleep 1; done",
            ready.display()
        );
        let mut r = Running::spawn(&spec(dir.path(), &script)).unwrap();
        wait_until("子进程就绪(已忽略 TERM)", || ready.exists());
        let started = Instant::now();
        let status = r.stop(Duration::from_millis(300)).unwrap();
        assert!(
            started.elapsed() >= Duration::from_millis(300),
            "先给了宽限期"
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!status.success());
    }

    #[test]
    fn dropping_a_running_process_does_not_leave_it_behind() {
        let dir = tempfile::tempdir().unwrap();
        let r = Running::spawn(&spec(dir.path(), "sleep 300")).unwrap();
        let pid = r.pid() as i32;
        assert!(alive(pid));
        drop(r);
        wait_until("被 Drop 掉的进程结束", || !alive(pid));
    }

    #[test]
    fn a_missing_program_and_an_empty_argv_are_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = spec(dir.path(), "");
        s.argv = vec!["definitely-not-a-program-xyz".into()];
        assert!(Running::spawn(&s).is_err());
        s.argv = vec![];
        assert_eq!(
            Running::spawn(&s).err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn free_ports_are_loopback_bindable() {
        // 选出来到绑定之间有竞争窗口(文档里写明了),并行的别的测试可能恰好抢走:重试几次再下结论。
        let bindable = (0..20).any(|_| {
            let p = pick_free_port().unwrap();
            TcpListener::bind(("127.0.0.1", p)).is_ok()
        });
        assert!(bindable);
    }

    #[test]
    fn a_leftover_process_with_a_matching_command_line_is_killed_on_startup() {
        let dir = tempfile::tempdir().unwrap();
        let argv: Vec<String> = ["sleep".into(), "301".into()].to_vec();
        // 模拟"dozerd 被强杀后留下的孤儿":自己起一个独立进程组的 sleep,不经 Running(没人会 stop 它)
        let child = Command::new("sleep")
            .arg("301")
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id();
        write_record(dir.path(), &RunRecord { pid, argv }).unwrap();
        let mut child = child;
        assert_eq!(
            reap_orphan(dir.path(), Duration::from_secs(1)).unwrap(),
            Reaped::Killed(pid)
        );
        let _ = child.wait();
        assert!(!alive(pid as i32));
        assert!(!record_path(dir.path()).exists(), "记录被清掉");
    }

    /// pid 被系统复用给了无关进程:命令行对不上就**不能杀**。
    #[test]
    fn a_process_with_a_different_command_line_is_never_killed() {
        let dir = tempfile::tempdir().unwrap();
        let mut innocent = Command::new("sleep")
            .arg("302")
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = innocent.id();
        write_record(
            dir.path(),
            &RunRecord {
                pid,
                argv: vec!["node".into(), "server.js".into()],
            },
        )
        .unwrap();
        assert_eq!(
            reap_orphan(dir.path(), Duration::from_millis(200)).unwrap(),
            Reaped::Stale
        );
        assert!(alive(pid as i32), "无关进程不能被杀");
        assert!(!record_path(dir.path()).exists());
        innocent.kill().unwrap();
        let _ = innocent.wait();
    }

    #[test]
    fn missing_dead_and_corrupt_records_are_handled() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            reap_orphan(dir.path(), Duration::from_millis(100)).unwrap(),
            Reaped::Nothing
        );
        // 已经退出的进程
        let mut gone = Command::new("true").spawn().unwrap();
        let pid = gone.id();
        gone.wait().unwrap();
        write_record(
            dir.path(),
            &RunRecord {
                pid,
                argv: vec!["true".into()],
            },
        )
        .unwrap();
        assert_eq!(
            reap_orphan(dir.path(), Duration::from_millis(100)).unwrap(),
            Reaped::Stale
        );
        std::fs::write(record_path(dir.path()), "{not json").unwrap();
        assert_eq!(
            reap_orphan(dir.path(), Duration::from_millis(100)).unwrap(),
            Reaped::Stale
        );
        assert!(!record_path(dir.path()).exists());
    }

    #[test]
    fn command_lines_are_compared_as_joined_argv() {
        assert!(command_matches(
            "node server.js --port 1",
            &[
                "node".into(),
                "server.js".into(),
                "--port".into(),
                "1".into()
            ]
        ));
        assert!(!command_matches("node server.js", &["node".into()]));
    }
}
```

`process/mod.rs`(完整最终内容):

```rust
//! 进程型应用的**监管核心**(bytehost A6a):把一个 Node/Python 应用当作子进程跑起来、盯着、停掉——
//! 只有机制,**不接** `AppManager`、gateway、协议(那是 A6b/A6c)。拆成互相独立、可单测的小块:
//!
//! - `restart`:崩溃后的重启退避与放弃(纯函数);
//! - `env`:给子进程的**白名单环境**(不继承 dozerd 的密钥/令牌)与端口环境变量名校验(纯函数);
//! - `log`:有大小上限、会轮转的日志文件与"把子进程输出泵进去"的线程;
//! - `health`:阻塞式 HTTP 健康探测(进程活着 ≠ 服务就绪);
//! - `supervise`:启动(独立进程组)、存活检查、优雅停止(SIGTERM→宽限→SIGKILL,整组)与孤儿清理。

pub mod env;
pub mod health;
pub mod log;
pub mod restart;
#[cfg(unix)]
pub mod supervise;

/// 端到端:真起一个 Python 静态服务器,走 `build_env` 的端口变量 → 健康探测 → 停止,端口随之关闭。
/// 机器上没有 `python3` 时跳过(开发机都有;不为测试引入别的依赖)。
#[cfg(all(test, unix))]
mod e2e {
    use super::env::{EnvSpec, build_env};
    use super::health::{Health, probe_http, wait_healthy};
    use super::supervise::{ProcessSpec, Running, pick_free_port};
    use crate::id::AppId;
    use std::time::Duration;

    #[test]
    fn a_python_server_is_started_probed_healthy_and_stopped_with_its_port_closing() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("跳过:没有 python3");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<h1>hi</h1>").unwrap();
        let port = pick_free_port().unwrap();
        let id = AppId::new("pytest-app").unwrap();
        let data = dir.path().join("data");
        let mut env = build_env(
            std::env::vars_os(),
            &EnvSpec {
                app_id: &id,
                port,
                port_env: "APP_PORT",
                data_dir: &data,
            },
        );
        // 子进程要找得到 python3:父 PATH 已在白名单里。
        let _ = &mut env;
        let mut r = Running::spawn(&ProcessSpec {
            argv: vec![
                "python3".into(),
                "-c".into(),
                "import os,http.server,socketserver; \
                 socketserver.TCPServer.allow_reuse_address=True; \
                 http.server.test(HandlerClass=http.server.SimpleHTTPRequestHandler, port=int(os.environ['APP_PORT']), bind='127.0.0.1')"
                    .into(),
            ],
            cwd: dir.path().to_path_buf(),
            env,
            log_path: dir.path().join("logs/app.log"),
            log_max_bytes: 1 << 20,
            log_keep: 2,
        })
        .unwrap();
        let health = wait_healthy(
            port,
            "/",
            Duration::from_secs(15),
            Duration::from_millis(100),
            || r.try_exit().ok().flatten().is_none(),
        );
        assert_eq!(
            health,
            Health::Healthy,
            "应用没起来;日志: {:?}",
            std::fs::read_to_string(dir.path().join("logs/app.log"))
        );
        r.stop(Duration::from_secs(3)).unwrap();
        assert!(
            matches!(
                probe_http(port, "/", Duration::from_millis(500)),
                Health::Unhealthy(_)
            ),
            "停止后端口应已关闭"
        );
    }
}
```

- [ ] **Step 4: GREEN。** `cargo fmt -p bytehost-apps && cargo test -p bytehost-apps --all-features process::` → 31 个全过,约 1 秒。
- [ ] **Step 5: 变异检查(串行,附录脚本)。** `supervise` 的 3 条(杀组改成杀单个进程、命令行核对恒真、日志泵收尾超时改成 60 秒)必须各自让对应测试 FAILED,**且失败要快**(分别约 6 秒/1 秒/20 秒,而不是挂起)。
- [ ] **Step 6: 全量门禁。** `cargo fmt --check -p bytehost-apps && cargo clippy -p bytehost-apps --all-features --all-targets && cargo clippy -p bytehost-apps --all-targets && bash scripts/check-bytehost-apps-deps.sh && cargo test -p bytehost-apps && cargo test -p bytehost-apps --all-features` → 全过(默认 feature 64 个、全 feature 186 个);clippy 无警告。
- [ ] **Step 7: 稳定性。** 并行把 `process::` 连跑 30 遍,0 失败。
- [ ] **Step 8: Commit。** `git add crates/bytehost-apps && git commit -m "feat(bytehost-apps): process supervision — spawn in own group, graceful group stop, orphan reaping (A6a task 2)"`

### Task 3: 文档

**Files:** Modify 规格(A6 切分)、`CLAUDE.md`、本计划(执行后修订)

- [ ] **Step 1:** 规格 §7 表后加一行"**A6(二期)** 进程型运行时,切分为 A6a–A6d,见 `plans/2026-10-07-bytehost-a6a-process-supervisor.md`";`CLAUDE.md` 的 `bytehost-apps` 行补一句"`process` 模块是进程型应用的监管核心(白名单环境、独立进程组、有上限的日志、孤儿清理),A6a 只有机制,未接 `AppManager`"。
- [ ] **Step 2: Commit。** `git commit -am "docs(bytehost): A6 split and process module note (A6a task 3)"`

## 已知局限(写在这里,不是缺陷)

- **未接线**:A6a 合并后没有任何调用方;`validate_port_env` 尚无人调用(A6c 必须调,见 Review Focus)。
- **运行时二进制怎么找**:dozerd 若由 GUI 拉起,它的 `PATH` 往往只有 `/usr/bin:/bin`——找不到 `node`/`uv`/`python3`。白名单环境会原样传这个 `PATH`。A6c 必须用探测结果/A6d 装好的固定路径来改写 `argv[0]` 或 `PATH`,不能依赖 dozerd 继承的 `PATH`。
- **白名单偏窄**:没放 `SHELL`、`XDG_*`、`NVM_DIR`、`UV_*`、`npm_config_*`、代理变量等;某个运行时需要什么由 A6c 按运行时显式加,不在这里放宽。
- **`reap_orphan` 依赖 `ps`**:macOS/Linux 的 `ps -ww -o command=` 都可用;含空格的参数两边对不上就不杀(安全方向的偏差)。
- **端口竞争窗口**:`pick_free_port` 先绑后放,到子进程真正绑定之间有极小窗口;调用方把"子进程起不来"当普通启动失败处理。
- **日志上限是软的**:单次写入超过 `max_bytes` 时不拆分(整块写进一个文件)。
- **非 unix**:`supervise` 仅 `#[cfg(unix)]`;其余平台暂无监管实现(项目当前只发 macOS)。
- **没有沙箱**:子进程以 dozerd 同一用户身份运行,能读写用户的文件、联网(强制等级为 `Advisory`,见 `runtime::enforcement_for`)。白名单环境只防"继承密钥",不防应用自己去读 `~/.ssh`。
- **并发做变异检查会互相踩文件**:本计划原型阶段就因为两个后台脚本同时改同一个文件而得到过假的"测试挂起"。变异检查必须**串行**,且每次从快照恢复。

## 附录:变异脚本(执行辅助,不提交)

先把 `crates/bytehost-apps/src/process/` 完整复制到 `/tmp/a6-golden/process`,再运行;脚本每条都从快照恢复、串行执行,最后再恢复一次。`restart`/`env`/`log`/`health` 的条目在 Task 1 之后就能跑,`supervise` 的在 Task 2 之后。

```bash
#!/bin/bash
# 逐条变异检查:每次都从 golden 恢复,串行运行,不并发。
G=/tmp/a6-golden/process
D=$HOME/Projects/CoralProjects/byteboy/dozer-a6-probe/crates/bytehost-apps
cd $D
run(){ # file, from, to, testfilter
  cp $G/* src/process/ 2>/dev/null
  python3 - "$1" "$2" "$3" <<'PY'
import sys
f,a,b=sys.argv[1:]
p="src/process/"+f
s=open(p).read()
if a not in s:
    print("PATTERN-NOT-FOUND:",a[:60]); sys.exit(3)
open(p,"w").write(s.replace(a,b,1))
PY
  [ $? -ne 0 ] && return
  echo "== $1: $2"
  RUSTC_WRAPPER= cargo test -q -p bytehost-apps --all-features "$4" 2>&1 | grep -E "FAILED|test result" | head -3
}
run restart.rs "if n > policy.max_restarts {" "if n >= policy.max_restarts {" process::restart
run restart.rs "if ran_for >= policy.stable_after {" "if false {" process::restart
run env.rs "PASS_EXACT.contains(&k.as_ref()) || PASS_PREFIX.iter().any(|p| k.starts_with(p))" "true" process::env
run env.rs "if RESERVED.contains(&name)" "if false && RESERVED.contains(&name)" process::env
run log.rs "if self.written > 0 && self.written + buf.len() as u64 > self.max_bytes {" "if false {" process::log
run health.rs "Some(code) if code < 500 => Health::Healthy," "Some(code) if code <= 503 => Health::Healthy," process::health
run supervise.rs "let r = unsafe { libc::kill(-pgid, sig) };" "let r = unsafe { libc::kill(pgid, sig) };" process::supervise::tests::stop_terminates
run supervise.rs "actual == argv.join(\" \")" "true" process::supervise
run supervise.rs "const PUMP_JOIN_TIMEOUT: Duration = Duration::from_secs(1);" "const PUMP_JOIN_TIMEOUT: Duration = Duration::from_secs(60);" process::supervise::tests::stop_does_not_hang
cp $G/* src/process/ 2>/dev/null
echo SWEEP_DONE
```
