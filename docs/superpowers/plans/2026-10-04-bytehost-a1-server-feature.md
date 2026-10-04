# bytehost 应用宿主 A1:`server` feature——`AppManager`、`static_web` runtime、gateway、端口持久化、运行时探测 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地应用宿主规格(`docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`)§5、§6、§7 的 **A1** 切片:在 `bytehost-apps` 里加 `server` feature——一个本机 HTTP **gateway**(一个固定端口、按 `Host` 头路由到各应用站点,带 Host 校验、会话令牌、路径解析、按授予权限生成的 CSP)、`AppManager`(安装/启动/停止/卸载/启动对账,只支持 `static_web`)、gateway 端口的随机选取与持久化、docker/node/python 的分层可用性探测与各 runtime 的强制等级表。**不接任何现有 crate**(dozerd 接线是 A2),只改 `bytehost-apps` 自己、`Cargo.lock`、两份文档与 `CLAUDE.md` 一行。

**Architecture:** 一个代码任务 + 一个文档任务。Task 1 测试先行:先写 6 个模块的测试与测试辅助 `testutil.rs`、改 `Cargo.toml`/`lib.rs` 并建只含测试的骨架(编译失败,RED),再跑装配脚本写入实现,逐个 feature 组合测试、clippy、依赖门禁,16 个变异检验证明测试会失败。Task 2 回填文档。

**Tech Stack:** Rust(`bytehost-apps`,edition 2024);`server` feature 新增依赖 `tokio`(workspace 已有)、`hyper 1`(`server`+`http1`)、`hyper-util`(`tokio`)、`http-body-util`、`bytes`、`uuid`(workspace 已有)——**用户 2026-10-04 已确认这套 HTTP 栈(A11)**,它们在 `Cargo.lock` 里都已存在,只新增一个传递依赖 `httpdate`。默认 feature 仍然只有 serde/serde_json。Python 3 标准库(一次性脚本)。

**Spec:** 应用宿主规格 §5.2(gateway 方案与端口策略,A1/A12 已定)、§6.1(gateway 放进 dozerd,A7 已定)、§6.2(运行时探测)、§7(切片 A1);路线图 §3 阶段 1。**V1 已实测通过**(同端口多应用,见 `spike/origin-gateway/README.md`)。

## Global Constraints

- **`bytehost-apps` 默认 feature 的依赖闭包不得变**:只有 serde/serde_json 家族;`server` 之外的 feature 不得引入 tokio/hyper。`scripts/check-bytehost-apps-deps.sh` 守住(本计划不改它,Task 1 里只是再跑一遍)。**任何 feature 组合下都不得出现 `dozer*` 依赖。**
- **本 crate 里不得出现 `dozer`/`Dozer` 字样的类型或路径。**
- **在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytehost-a1/...`),分支 `bytehost-a1`,从当前 `main` 开。主 checkout 常有并发会话与未提交改动(此刻工作区里就有别的会话的 PlantUML 文档),不得在其上改代码或 `git add -A`;只 `git add` 指定路径;提交前 `git status` 与 `git diff --cached --stat` 看全貌。
- **worktree 里不要复制 `.cargo/config.toml`**(同 A0):`Cargo.lock` 保持干净,新增的 13 行直接提交。提交前 `git diff Cargo.lock` 确认**没有任何 `-` 行**,新增的是 `bytehost-apps` 块里的 6 个依赖名和一个 `httpdate` 包。
- **变异检验前先 `git add` 暂存当前版本**(新文件也要 `git add`),变异后用 `git checkout -- crates` 还原。
- **每次改 Rust 后跑 `cargo fmt -p bytehost-apps`;变异脚本匹配的是 `cargo fmt` 之后的源码。**
- **`cargo` 命令若报 `sccache: Operation not permitted`:** 那是主 checkout 里的环境问题,worktree 里不会遇到;万一遇到,加 `RUSTC_WRAPPER=` 前缀。
- **跑测试一律同时在默认并行度下跑一遍**(不要只用 `--test-threads=1`):草稿里正是默认并行度暴露了一个只在特定调度下才出现的挂起(见 Review Focus 1)。
- **heredoc 一律用带引号的分隔符(`<<'EOF'`);或直接用编辑器/Write 工具写文件。**
- 提交信息用 `feat:`/`test:`/`docs:` 前缀;末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知基线

- 本切片不改任何现有 crate:**全 workspace 的既有测试结果不变**。只证明新 crate 自身。
- 草稿结果(`cargo test -p bytehost-apps`):默认 feature **55**;`--features digest` **63**;`--features manifest-toml` **61**;`--features server` 与 `--all-features` 各 **118**(`server` 比"digest+toml 全开"多 49 个:gateway 12 + 静态文件 6 + 端口 5 + runtime 表 4 + 探测 7 + manager 15)。clippy 在默认与 `--all-features` 下均**零诊断**。

## Review Focus

1. **关停会丢通知(草稿里真实出现过):** 第一版 `Gateway::shutdown` 用 `Notify::notify_waiters`,它只唤醒"此刻正在等"的任务——accept 循环还没跑到 `notified()` 时通知就丢了,`task.await` 永远挂起(默认并行度下 2 个测试挂住)。已改成 `notify_one`(存一个许可),回归测试 `shutdown_never_hangs_even_when_called_before_the_accept_loop_first_runs` 在 `current_thread` 运行时里**确定性**复现(变异 12 证明)。另外 `shutdown` 只停 accept 循环,**已建立的连接不会被主动掐断**(没有优雅排空)——对本机静态站点可以接受,A2 接 dozerd 退出流程时要知道。
2. **令牌是整个 gateway 进程一把,不是每应用一把:** 所有应用共用同一个令牌值;隔离靠的是 Cookie 是 **host-only**(各应用主机名不同,浏览器不会把 A 的 Cookie 发给 B)与 `data_store_identifier`。令牌每次 gateway 启动重新生成(`uuid` v4 ×2,64 位十六进制),**不持久化**:dozerd 重启后 GUI 必须重新要 `launch_url`。`launch_url` 带秘密,不得写日志、不得进广播事件(测试 `install_start_serve_and_stop_round_trip_with_events` 断言事件里没有 `bh_token`)。
3. **gateway 的能力边界:** 只有 HTTP/1.1、只有 GET/HEAD、整文件读进内存(无 Range、无 ETag/条件请求、无流式)、没有请求体大小/慢连接/并发数限制、没有 WebSocket 与反向代理(进程型应用的 gateway 能力属于后续切片)。静态 Web 应用(Excalidraw 构建产物几 MB)足够;A5 端到端验收才会真正检验。
4. **CSP 可能过严:** `outbound = none` 时 `script-src 'self' 'wasm-unsafe-eval'`——**不允许内联脚本**;`style-src` 允许 `'unsafe-inline'`。真实应用(Excalidraw)是否能在这个 CSP 下正常工作**没有被验证**(规格 V2,A5 才测);若有应用需要内联脚本,正确做法是它在 manifest 里申请更宽的网络权限,而不是放宽 CSP。
5. **安装顺序:先拷贝再决定(草稿里改过一次)。** 第一版先读源目录的 manifest 来决定 staging 位置、并把**源目录里的** manifest 文本写进注册表,这在"读 manifest"与"拷贝"之间留了一个窗口:源目录被换掉时,落盘的 `manifest.toml` 可能不是被 `verify` 过的那份。现在 `install` 只做一件读源目录的事——拷贝到 `apps/.staging-<随机>/`;之后的一切(manifest、摘要、id、版本、落盘的 manifest 文本)都来自 staging 副本。每个失败出口都清掉 staging(测试 `every_failed_install_cleans_up_and_the_saved_manifest_is_the_staged_one`,变异 9)。**源目录在"拷贝"这一刻的内容就是被审批的内容的前提,仍然依赖 `install_plan` 时算的摘要与 `install` 时 staging 的摘要一致——不一致就被 `verify` 拒绝。**
6. **`AppManager` 的并发假设:** 所有改状态的方法用同一把 `Mutex`(单写者);`list()` 不拿锁(读的是落盘记录,可能读到某次写入的中间状态——`write_atomic` 保证不会读到半个文件)。一期的 `install`/`start` 都是同步的、瞬时的;规格 §4.3 的任务句柄/进度事件留到有耗时 runtime 时再加,`AppEvent::Progress` 类型已存在但本切片不发。
7. **`reconcile` 的语义:** 先 `recover_after_supervisor_restart` 修正陈旧的观察态并落盘,再按 `desired` 走一步 `next_action`;`Uninstall` 只卸程序(`UninstallMode::Program`),**不会**替用户删数据。一个应用失败不影响其他应用,失败列表返回给调用方。`Running` + 可重试的 `Failed` 每次 `reconcile` 都会再试一次 `Start`——**没有退避/次数上限**(A0 评审已记为 A1 的 manager 要自己限制;本切片只在 supervisor 启动时调一次 `reconcile`,不会紧循环,A2 若加周期性对账必须带退避)。
8. **探测不执行任何有副作用的命令**(只有 `--version`/`docker info`),且带 5 秒超时、超时即杀子进程;但 `docker info` 在 Docker Desktop 首次启动期可能较慢,会被报成 `Unavailable("… 执行超时")`——这是"当前不可用",不是"没装"。
9. **范围外:** 压缩包/仓库来源、Node/Python/容器 runtime 的实际运行、WebSocket 代理、dozerd 接线与 `proto` 类型(A2)、GUI(A4)、Excalidraw 端到端(A5)。

---

## File Structure

| 路径 | 职责 | 任务 |
|---|---|---|
| `crates/bytehost-apps/Cargo.toml` | 新增 `server` feature 与它的可选依赖;dev-dependency 加 `tokio` | 1 |
| `crates/bytehost-apps/src/lib.rs` | 声明 `gateway`/`manager`/`port`/`runtime`(feature `server`)与测试辅助 | 1 |
| `crates/bytehost-apps/src/testutil.rs`(新,仅测试) | 原始 TCP HTTP 客户端(精确控制 `Host`/`Cookie`) | 1 |
| `crates/bytehost-apps/src/gateway/static_files.rs`(新) | 百分号解码、路径解析(安全边界)、MIME + 6 个测试 | 1 |
| `crates/bytehost-apps/src/gateway/mod.rs`(新) | `Gateway`、`respond`(Host → 令牌 → 站点 → 方法 → 路径)、`csp_for` + 12 个测试 | 1 |
| `crates/bytehost-apps/src/port.rs`(新) | 端口随机选取与持久化 + 5 个测试 | 1 |
| `crates/bytehost-apps/src/runtime/mod.rs`(新) | `enforcement_for` 强制等级表 + 4 个测试 | 1 |
| `crates/bytehost-apps/src/runtime/probe.rs`(新) | `CommandRunner`/`SystemRunner`、docker/node/python 分层探测 + 7 个测试 | 1 |
| `crates/bytehost-apps/src/manager.rs`(新) | `AppManager`:`install_plan`/`install`/`start`/`stop`/`uninstall`/`list`/`reconcile` + 15 个测试 | 1 |
| `Cargo.lock` | `bytehost-apps` 块多 6 个依赖名,多一个 `httpdate` 包(13 行,无删除) | 1 |
| `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`、`…-extraction-roadmap.md`、`CLAUDE.md` | 回填 | 2 |

一次性脚本与测试/实现源文件(**不提交**,最终内容在 crate 里)放 `$SCRATCH`(仓库外)。

---

### Task 1: `server` feature

**Files:** 见上表。

**Interfaces:**
- Produces(A2/A4 依赖),均 `pub`,feature `server`:
  - `gateway::{Gateway, GatewayConfig { port: u16 }, GatewayError::{PortInUse(u16), Io}, csp_for(&Permissions) -> Option<String>}`;`Gateway::{start(GatewayConfig) async -> Result<Gateway, GatewayError>, port(), add_site(&AppId, PathBuf, Option<String>), remove_site(&AppId) -> bool, has_site(&AppId) -> bool, site_url(&AppId) -> String /*不含令牌*/, launch_url(&AppId) -> String /*含令牌,秘密*/, shutdown(self) async}`。
  - `port::{PORT_MIN, PORT_MAX, pick_port() -> u16, load_or_choose_port(&Path) -> io::Result<u16>}`。
  - `runtime::{enforcement_for(&Runtime) -> Vec<EnforcementEntry>, RuntimeAvailability::{Available{detail}, Unavailable{detail}, NotInstalled}, CommandRunner, SystemRunner, CommandOutput, CommandError, probe_docker/probe_node/probe_python(&dyn CommandRunner), probe_all(&dyn CommandRunner) -> Vec<(&'static str, RuntimeAvailability)>}`。
  - `manager::{AppManager, AppSource::LocalDir(PathBuf), AppSummary { id, name, version, desired, observed, url }, ManagerError}`;`AppManager::{new(root, Version, Arc<Gateway>) -> io::Result<_>, events() -> broadcast::Receiver<AppEvent>, launch_url(&AppId) -> String, install_plan(&AppSource, Provenance, TrustLevel) -> Result<InstallPlan, ManagerError>, install(&ApprovedInstallPlan, &AppSource, now_ms: u64) -> Result<(), ManagerError>, start(&AppId) -> Result<String, ManagerError>, stop(&AppId), uninstall(&AppId, UninstallMode), list() -> Result<Vec<AppSummary>, ManagerError>, reconcile() -> Vec<(AppId, ManagerError)>}`。

- [ ] **Step 1: 建 worktree、分支、脚本目录**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytehost-a1 -b bytehost-a1 main
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a1 && git log --oneline | head -1 && git status --short && ls .cargo 2>/dev/null
export SCRATCH=~/Projects/CoralProjects/byteboy/dozer-bytehost-a1-scratch && mkdir -p $SCRATCH/a1
```

Expected: 干净、分支 `bytehost-a1`、`ls .cargo` 没有输出(**不要**复制 `config.toml`)。**此后所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytehost-a1/` 前缀,且每个新 shell 先 `export SCRATCH=…`。**

- [ ] **Step 2: 确认起点**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a1 && cargo test -p bytehost-apps --all-features 2>&1 | grep -E "^error|test result: .* [1-9][0-9]* passed" && ./scripts/check-bytehost-apps-deps.sh`
Expected: `test result: ok. 69 passed`(A0 合并后 `--all-features` 的数量);门禁 `ok`。**若数字不是 69,说明 `main` 上 A0 之后有改动,以实测为准并在 Step 6 的期望里相应调整。**

- [ ] **Step 3: 写测试、辅助与骨架(先看编译失败)**

把下面 6 段测试写成 `$SCRATCH/a1/tests_<模块>.rs`(每段开头有一个空行,保留);再写 `$SCRATCH/a1/Cargo.toml`、`$SCRATCH/a1/lib.rs`、`$SCRATCH/a1/testutil.rs`:

`$SCRATCH/a1/tests_gateway_static_files.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::write_files;
    use std::fs;

    fn site() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("site");
        write_files(
            &root,
            &[
                ("index.html", "<h1>home</h1>"),
                ("assets/app.js", "1"),
                ("docs/index.html", "docs"),
                ("a b/c.txt", "space"),
            ],
        );
        write_files(tmp.path(), &[("secret.txt", "TOP SECRET")]);
        (tmp, root)
    }

    fn file(root: &Path, rel: &str) -> Resolved {
        Resolved::File(root.canonicalize().unwrap().join(rel))
    }

    #[test]
    fn percent_decoding_handles_valid_invalid_and_non_utf8_input() {
        assert_eq!(
            percent_decode("/a%20b/%E4%B8%AD").as_deref(),
            Some("/a b/中")
        );
        assert_eq!(percent_decode("/plain").as_deref(), Some("/plain"));
        assert_eq!(percent_decode("/%2e%2E").as_deref(), Some("/.."));
        for bad in ["/%", "/%a", "/%zz", "/%g1", "/%FF%FE"] {
            assert_eq!(percent_decode(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_root_and_directories_resolve_to_their_index_html() {
        let (_tmp, root) = site();
        assert_eq!(resolve(&root, "/"), file(&root, "index.html"));
        assert_eq!(resolve(&root, ""), file(&root, "index.html"));
        assert_eq!(resolve(&root, "/docs"), file(&root, "docs/index.html"));
        assert_eq!(resolve(&root, "/docs/"), file(&root, "docs/index.html"));
        assert_eq!(
            resolve(&root, "/assets/app.js"),
            file(&root, "assets/app.js")
        );
        assert_eq!(resolve(&root, "/a%20b/c.txt"), file(&root, "a b/c.txt"));
        assert_eq!(
            resolve(&root, "/./assets//app.js"),
            file(&root, "assets/app.js"),
            "空段与 . 段被忽略"
        );
    }

    #[test]
    fn missing_files_and_directories_without_an_index_are_not_found() {
        let (_tmp, root) = site();
        assert_eq!(resolve(&root, "/nope.html"), Resolved::NotFound);
        assert_eq!(
            resolve(&root, "/assets"),
            Resolved::NotFound,
            "assets/ 没有 index.html"
        );
    }

    #[test]
    fn no_spelling_of_a_parent_reference_can_leave_the_site_root() {
        let (_tmp, root) = site();
        for target in [
            "/../secret.txt",
            "/%2e%2e/secret.txt",
            "/%2E%2E/secret.txt",
            "/assets/../../secret.txt",
            "/assets/%2e%2e/%2e%2e/secret.txt",
            "/..",
        ] {
            assert_eq!(resolve(&root, target), Resolved::Forbidden, "{target}");
        }
        // 编码后的斜杠只是普通分隔符,不会绕过 .. 检查
        assert_eq!(resolve(&root, "/%2e%2e%2fsecret.txt"), Resolved::Forbidden);
        // 反斜杠、NUL、坏编码直接 400
        for target in ["/..%5csecret.txt", "/a%5cb", "/%00", "/%zz", "/%FF"] {
            assert_eq!(resolve(&root, target), Resolved::BadRequest, "{target}");
        }
        // 双重编码只是一个普通文件名,找不到
        assert_eq!(resolve(&root, "/%252e%252e/secret.txt"), Resolved::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_outside_the_root_is_forbidden() {
        let (tmp, root) = site();
        std::os::unix::fs::symlink(tmp.path().join("secret.txt"), root.join("leak.txt")).unwrap();
        std::os::unix::fs::symlink(tmp.path(), root.join("up")).unwrap();
        assert_eq!(resolve(&root, "/leak.txt"), Resolved::Forbidden);
        assert_eq!(resolve(&root, "/up/secret.txt"), Resolved::Forbidden);
        // 指向根内部的链接没问题
        std::os::unix::fs::symlink(root.join("assets/app.js"), root.join("inner.js")).unwrap();
        assert_eq!(resolve(&root, "/inner.js"), file(&root, "assets/app.js"));
        let _ = fs::remove_file(root.join("inner.js"));
    }

    #[test]
    fn mime_types_cover_what_web_apps_ship_and_default_to_octet_stream() {
        let m = |name: &str| mime_for(Path::new(name));
        assert_eq!(m("a.html"), "text/html; charset=utf-8");
        assert_eq!(m("A.HTM"), "text/html; charset=utf-8");
        assert_eq!(m("a.mjs"), "text/javascript; charset=utf-8");
        assert_eq!(m("a.css"), "text/css; charset=utf-8");
        assert_eq!(m("a.json"), "application/json");
        assert_eq!(m("a.svg"), "image/svg+xml");
        assert_eq!(m("a.woff2"), "font/woff2");
        assert_eq!(m("a.wasm"), "application/wasm");
        assert_eq!(m("a.png"), "image/png");
        assert_eq!(m("noext"), "application/octet-stream");
        assert_eq!(m("a.unknown"), "application/octet-stream");
    }
}
```

`$SCRATCH/a1/tests_gateway_mod.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{NetworkPerm, Outbound};
    use crate::testutil::{Req, write_files};

    const TOKEN: &str = "tok";

    fn state_with(port: u16, apps: &[(&str, &Path, Option<String>)]) -> State {
        let mut sites = HashMap::new();
        for (id, root, csp) in apps {
            sites.insert(
                id.to_string(),
                Site {
                    root: root.to_path_buf(),
                    csp: csp.clone(),
                },
            );
        }
        State {
            port,
            token: TOKEN.to_string(),
            sites: RwLock::new(sites),
        }
    }

    fn site_dir() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("site");
        write_files(
            &root,
            &[("index.html", "<h1>excalidraw</h1>"), ("app.js", "1")],
        );
        write_files(tmp.path(), &[("secret.txt", "TOP SECRET")]);
        (tmp, root)
    }

    fn get(state: &State, host: Option<&str>, target: &str, cookie: Option<&str>) -> StatusCode {
        respond(state, &Method::GET, host, target, cookie).status()
    }

    const COOKIE: &str = "bh_session=tok";

    #[test]
    fn the_host_header_must_be_exactly_a_valid_app_dot_localhost_with_the_gateways_port() {
        let (_tmp, root) = site_dir();
        let st = state_with(5000, &[("excalidraw", &root, None)]);
        assert_eq!(
            get(&st, Some("excalidraw.localhost:5000"), "/", Some(COOKIE)),
            StatusCode::OK
        );
        for bad in [
            Some("excalidraw.localhost:5001"),
            Some("excalidraw.localhost"),
            Some("evil.example:5000"),
            Some("excalidraw.localhost.evil.example:5000"),
            Some("127.0.0.1:5000"),
            Some("localhost:5000"),
            Some(".localhost:5000"),
            Some("a.b.localhost:5000"),
            Some("EXCALIDRAW.localhost:5000"),
            Some("excalidraw.localhost:5000:1"),
            Some("excalidraw.localhost:notaport"),
            Some(""),
            None,
        ] {
            assert_eq!(
                get(&st, bad, "/", Some(COOKIE)),
                StatusCode::MISDIRECTED_REQUEST,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn without_the_token_every_request_is_403_even_for_apps_that_do_not_exist() {
        let (_tmp, root) = site_dir();
        let st = state_with(5000, &[("excalidraw", &root, None)]);
        let host = Some("excalidraw.localhost:5000");
        assert_eq!(get(&st, host, "/", None), StatusCode::FORBIDDEN);
        assert_eq!(
            get(&st, host, "/", Some("bh_session=wrong")),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            get(&st, host, "/", Some("other=tok")),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            get(&st, host, "/", Some("bh_session=tokx")),
            StatusCode::FORBIDDEN
        );
        // 不存在的应用:没令牌同样 403(不泄露哪些应用已安装);有令牌才是 404
        assert_eq!(
            get(&st, Some("ghost.localhost:5000"), "/", None),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            get(&st, Some("ghost.localhost:5000"), "/", Some(COOKIE)),
            StatusCode::NOT_FOUND
        );
        // 多个 cookie 里取对的那个
        assert_eq!(
            get(&st, host, "/", Some("a=1; bh_session=tok; b=2")),
            StatusCode::OK
        );
    }

    #[test]
    fn the_token_in_the_query_becomes_a_host_only_strict_cookie_and_is_stripped_by_a_redirect() {
        let (_tmp, root) = site_dir();
        let st = state_with(5000, &[("excalidraw", &root, None)]);
        let host = Some("excalidraw.localhost:5000");
        let r = respond(
            &st,
            &Method::GET,
            host,
            "/app.js?x=1&bh_token=tok&y=2",
            None,
        );
        assert_eq!(r.status(), StatusCode::FOUND);
        assert_eq!(r.headers()[header::LOCATION], "/app.js?x=1&y=2");
        let cookie = r.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .to_string();
        assert!(cookie.starts_with("bh_session=tok;"), "{cookie}");
        for attr in ["HttpOnly", "SameSite=Strict", "Path=/"] {
            assert!(cookie.contains(attr), "{cookie}");
        }
        assert!(
            !cookie.to_ascii_lowercase().contains("domain"),
            "host-only,不设 Domain:{cookie}"
        );
        // 只有令牌时 Location 就是路径本身
        let r = respond(&st, &Method::GET, host, "/?bh_token=tok", None);
        assert_eq!(r.headers()[header::LOCATION], "/");
        // 错误令牌:403,且不下发 Cookie
        let r = respond(&st, &Method::GET, host, "/?bh_token=nope", None);
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert!(r.headers().get(header::SET_COOKIE).is_none());
        // 令牌换 Cookie 之前仍要先过 Host 校验
        let r = respond(
            &st,
            &Method::GET,
            Some("evil.example:5000"),
            "/?bh_token=tok",
            None,
        );
        assert_eq!(r.status(), StatusCode::MISDIRECTED_REQUEST);
    }

    #[test]
    fn files_are_served_with_the_right_type_length_and_hardening_headers() {
        let (_tmp, root) = site_dir();
        let st = state_with(5000, &[("excalidraw", &root, None)]);
        let r = respond(
            &st,
            &Method::GET,
            Some("excalidraw.localhost:5000"),
            "/",
            Some(COOKIE),
        );
        assert_eq!(r.status(), StatusCode::OK);
        let h = r.headers();
        assert_eq!(h[header::CONTENT_TYPE], "text/html; charset=utf-8");
        assert_eq!(h[header::CONTENT_LENGTH], "19");
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(h[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(h[header::CACHE_CONTROL], "no-cache");
        assert!(
            h.get(header::CONTENT_SECURITY_POLICY).is_none(),
            "没配 CSP 就不带"
        );
        let js = respond(
            &st,
            &Method::GET,
            Some("excalidraw.localhost:5000"),
            "/app.js",
            Some(COOKIE),
        );
        assert_eq!(
            js.headers()[header::CONTENT_TYPE],
            "text/javascript; charset=utf-8"
        );
    }

    #[test]
    fn traversal_attempts_return_403_or_400_and_never_the_secret() {
        let (_tmp, root) = site_dir();
        let st = state_with(5000, &[("excalidraw", &root, None)]);
        for target in [
            "/../secret.txt",
            "/%2e%2e/secret.txt",
            "/..%5csecret.txt",
            "/%00",
        ] {
            let s = get(&st, Some("excalidraw.localhost:5000"), target, Some(COOKIE));
            assert!(
                s == StatusCode::FORBIDDEN || s == StatusCode::BAD_REQUEST,
                "{target}: {s}"
            );
        }
        assert_eq!(
            get(
                &st,
                Some("excalidraw.localhost:5000"),
                "/nope",
                Some(COOKIE)
            ),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn only_get_and_head_are_allowed_and_head_carries_no_body() {
        let (_tmp, root) = site_dir();
        let st = state_with(5000, &[("excalidraw", &root, None)]);
        let host = Some("excalidraw.localhost:5000");
        for m in [
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::OPTIONS,
        ] {
            let r = respond(&st, &m, host, "/", Some(COOKIE));
            assert_eq!(r.status(), StatusCode::METHOD_NOT_ALLOWED, "{m}");
            assert_eq!(r.headers()[header::ALLOW], "GET, HEAD");
        }
        let head = respond(&st, &Method::HEAD, host, "/", Some(COOKIE));
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers()[header::CONTENT_LENGTH], "19", "长度照给");
    }

    #[test]
    fn the_csp_follows_the_granted_outbound_network_permission() {
        let none = Permissions::default();
        let csp = csp_for(&none).expect("outbound=none 要有 CSP");
        for needed in [
            "default-src 'self'",
            "connect-src 'self'",
            "script-src 'self'",
        ] {
            assert!(csp.contains(needed), "{csp}");
        }
        assert!(
            !csp.contains("http:") && !csp.contains("https:") && !csp.contains('*'),
            "{csp}"
        );
        let any = Permissions {
            network: NetworkPerm {
                outbound: Outbound::Any,
            },
            ..Permissions::default()
        };
        assert_eq!(csp_for(&any), None);

        let (_tmp, root) = site_dir();
        let st = state_with(5000, &[("excalidraw", &root, Some(csp.clone()))]);
        let r = respond(
            &st,
            &Method::GET,
            Some("excalidraw.localhost:5000"),
            "/",
            Some(COOKIE),
        );
        assert_eq!(r.headers()[header::CONTENT_SECURITY_POLICY], csp.as_str());
    }

    #[test]
    fn query_helpers_split_the_token_and_read_cookies_precisely() {
        assert_eq!(split_token(None), (None, None));
        assert_eq!(split_token(Some("bh_token=t")), (Some("t".into()), None));
        assert_eq!(
            split_token(Some("a=1&bh_token=t&b=2")),
            (Some("t".into()), Some("a=1&b=2".into()))
        );
        assert_eq!(split_token(Some("a=1&&b")), (None, Some("a=1&b".into())));
        assert_eq!(
            split_token(Some("xbh_token=t")),
            (None, Some("xbh_token=t".into())),
            "只认完整的参数名"
        );
        assert_eq!(
            cookie_value("a=1; bh_session=x; b=2", "bh_session"),
            Some("x")
        );
        assert_eq!(cookie_value("xbh_session=x", "bh_session"), None);
        assert!(ct_eq("abc", "abc") && !ct_eq("abc", "abd") && !ct_eq("abc", "ab"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_port_serves_several_apps_each_by_its_own_host_name() {
        let (_tmp_a, root_a) = site_dir();
        let tmp_b = tempfile::tempdir().unwrap();
        write_files(tmp_b.path(), &[("index.html", "<h1>drawio</h1>")]);
        let gw = Gateway::start(GatewayConfig { port: 0 }).await.unwrap();
        let (a, b) = (
            AppId::new("excalidraw").unwrap(),
            AppId::new("drawio").unwrap(),
        );
        gw.add_site(&a, root_a, None);
        gw.add_site(&b, tmp_b.path().to_path_buf(), None);
        let port = gw.port();
        let cookie = format!(
            "bh_session={TOKEN_PLACEHOLDER}",
            TOKEN_PLACEHOLDER = gw.state.token
        );

        let ra = Req::get(port, &format!("excalidraw.localhost:{port}"), "/")
            .cookie(&cookie)
            .send();
        assert_eq!((ra.status, ra.text().contains("excalidraw")), (200, true));
        let rb = Req::get(port, &format!("drawio.localhost:{port}"), "/")
            .cookie(&cookie)
            .send();
        assert_eq!((rb.status, rb.text().contains("drawio")), (200, true));
        // 伪造 Host:421;错误端口:421
        assert_eq!(
            Req::get(port, "evil.example", "/")
                .cookie(&cookie)
                .send()
                .status,
            421
        );
        assert_eq!(
            Req::get(
                port,
                &format!("excalidraw.localhost:{}", port.wrapping_add(1)),
                "/"
            )
            .cookie(&cookie)
            .send()
            .status,
            421
        );
        // 没有 Cookie:403
        assert_eq!(
            Req::get(port, &format!("excalidraw.localhost:{port}"), "/")
                .send()
                .status,
            403
        );

        // 用 launch_url 里的令牌换 Cookie 再访问
        let launch = gw.launch_url(&a);
        let target = launch
            .strip_prefix(&format!("http://excalidraw.localhost:{port}"))
            .unwrap()
            .to_string();
        let r = Req::get(port, &format!("excalidraw.localhost:{port}"), &target).send();
        assert_eq!(r.status, 302);
        assert!(r.header("set-cookie").unwrap().starts_with("bh_session="));
        assert!(!gw.site_url(&a).contains("bh_token"), "site_url 不含令牌");

        // 注销之后 404(带 Cookie)
        assert!(gw.remove_site(&a));
        assert!(!gw.remove_site(&a));
        assert_eq!(
            Req::get(port, &format!("excalidraw.localhost:{port}"), "/")
                .cookie(&cookie)
                .send()
                .status,
            404
        );
        let head = Req::get(port, &format!("drawio.localhost:{port}"), "/")
            .cookie(&cookie)
            .method("HEAD")
            .send();
        assert_eq!((head.status, head.body.len()), (200, 0));
        gw.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_port_that_is_already_taken_is_a_clear_error_never_a_silent_new_port() {
        let first = Gateway::start(GatewayConfig { port: 0 }).await.unwrap();
        let taken = first.port();
        match Gateway::start(GatewayConfig { port: taken }).await {
            Err(GatewayError::PortInUse(p)) => assert_eq!(p, taken),
            Err(e) => panic!("{e}"),
            Ok(_) => panic!("不应该成功"),
        }
        first.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tokens_are_random_per_gateway_and_launch_urls_carry_them() {
        let a = Gateway::start(GatewayConfig { port: 0 }).await.unwrap();
        let b = Gateway::start(GatewayConfig { port: 0 }).await.unwrap();
        let id = AppId::new("excalidraw").unwrap();
        let (ua, ub) = (a.launch_url(&id), b.launch_url(&id));
        assert_ne!(a.state.token, b.state.token);
        assert_eq!(a.state.token.len(), 64);
        assert!(ua.contains(&format!("bh_token={}", a.state.token)));
        assert_ne!(ua, ub);
        assert!(!a.has_site(&id));
        a.shutdown().await;
        b.shutdown().await;
    }

    /// `Notify::notify_waiters` 只唤醒"此刻正在等"的任务:如果 `shutdown` 在 accept 循环第一次被轮询之前
    /// 就调用(`current_thread` 运行时里必然如此),通知会丢失,`task.await` 永远等下去。
    #[tokio::test]
    async fn shutdown_never_hangs_even_when_called_before_the_accept_loop_first_runs() {
        for _ in 0..20 {
            let gw = Gateway::start(GatewayConfig { port: 0 }).await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), gw.shutdown())
                .await
                .expect("shutdown 必须返回");
        }
    }
}
```

`$SCRATCH/a1/tests_port.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picked_ports_stay_inside_the_dynamic_range() {
        for _ in 0..2000 {
            let p = pick_port();
            assert!((PORT_MIN..=PORT_MAX).contains(&p), "{p}");
        }
    }

    #[test]
    fn the_first_call_chooses_and_persists_and_later_calls_return_the_same_port() {
        let tmp = tempfile::tempdir().unwrap();
        let first = load_or_choose_port(tmp.path()).unwrap();
        assert!((PORT_MIN..=PORT_MAX).contains(&first));
        assert!(tmp.path().join("gateway.json").is_file());
        for _ in 0..5 {
            assert_eq!(load_or_choose_port(tmp.path()).unwrap(), first);
        }
        assert!(
            !tmp.path().join("gateway.tmp").exists(),
            "原子写不留临时文件"
        );
    }

    #[test]
    fn a_missing_root_directory_is_created() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("a/b");
        let port = load_or_choose_port(&root).unwrap();
        assert_eq!(load_or_choose_port(&root).unwrap(), port);
    }

    #[test]
    fn corrupt_or_out_of_range_settings_are_errors_not_a_silent_new_port() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gateway.json");
        for bad in [
            "{not json",
            r#"{"port": 80}"#,
            r#"{"port": 50000, "extra": 1}"#,
            r#"{"port": 70000}"#,
            "{}",
        ] {
            fs::write(&path, bad).unwrap();
            let err = load_or_choose_port(tmp.path()).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{bad}");
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                bad,
                "坏文件原样保留,不被覆盖"
            );
        }
    }

    #[test]
    fn an_explicitly_persisted_port_is_returned_as_is() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("gateway.json"), r#"{"port": 51234}"#).unwrap();
        assert_eq!(load_or_choose_port(tmp.path()).unwrap(), 51234);
    }
}
```

`$SCRATCH/a1/tests_runtime_mod.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ContainerHttp, ProcessHttp};

    fn level(entries: &[EnforcementEntry], key: PermissionKey) -> Enforcement {
        entries
            .iter()
            .find(|e| e.key == key)
            .expect("每条权限都有一项")
            .enforcement
    }

    #[test]
    fn every_runtime_reports_every_permission_exactly_once() {
        let runtimes = [
            Runtime::StaticWeb {
                source: "web/".into(),
            },
            Runtime::Node {
                command: vec!["node".into()],
                lockfile: None,
                node: None,
                http: ProcessHttp {
                    port_env: "PORT".into(),
                },
            },
            Runtime::Python {
                command: vec!["python".into()],
                lockfile: None,
                python: None,
                http: ProcessHttp {
                    port_env: "PORT".into(),
                },
            },
            Runtime::Container {
                image: format!("x@sha256:{}", "a".repeat(64)),
                http: ContainerHttp { container_port: 80 },
            },
        ];
        for r in &runtimes {
            let entries = enforcement_for(r);
            assert_eq!(entries.len(), PermissionKey::ALL.len(), "{}", r.kind_name());
            for key in PermissionKey::ALL {
                assert_eq!(
                    entries.iter().filter(|e| e.key == key).count(),
                    1,
                    "{} {key:?}",
                    r.kind_name()
                );
            }
        }
    }

    #[test]
    fn static_web_enforces_network_with_csp_and_is_honest_about_the_rest() {
        let e = enforcement_for(&Runtime::StaticWeb {
            source: "web/".into(),
        });
        assert_eq!(
            level(&e, PermissionKey::NetworkOutbound),
            Enforcement::Enforced
        );
        assert_eq!(
            level(&e, PermissionKey::FilesystemData),
            Enforcement::Unsupported
        );
        for key in [
            PermissionKey::Clipboard,
            PermissionKey::Downloads,
            PermissionKey::Popups,
        ] {
            assert_eq!(level(&e, key), Enforcement::Advisory, "{key:?}");
        }
    }

    #[test]
    fn process_runtimes_never_claim_isolation() {
        let node = Runtime::Node {
            command: vec!["node".into()],
            lockfile: None,
            node: None,
            http: ProcessHttp {
                port_env: "PORT".into(),
            },
        };
        assert!(
            enforcement_for(&node)
                .iter()
                .all(|e| e.enforcement == Enforcement::Advisory)
        );
    }

    #[test]
    fn containers_enforce_mounts_but_not_outbound_network() {
        let c = Runtime::Container {
            image: format!("x@sha256:{}", "a".repeat(64)),
            http: ContainerHttp { container_port: 80 },
        };
        let e = enforcement_for(&c);
        assert_eq!(
            level(&e, PermissionKey::FilesystemData),
            Enforcement::Enforced
        );
        assert_eq!(
            level(&e, PermissionKey::NetworkOutbound),
            Enforcement::Advisory
        );
    }
}
```

`$SCRATCH/a1/tests_runtime_probe.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 按 "program arg1 arg2" 查表的假执行器。
    struct Fake(HashMap<String, Result<CommandOutput, CommandError>>);

    impl Fake {
        fn new(entries: Vec<(&str, Result<CommandOutput, CommandError>)>) -> Self {
            Self(
                entries
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            )
        }
    }

    impl CommandRunner for Fake {
        fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, CommandError> {
            let key = std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            self.0
                .get(&key)
                .cloned()
                .unwrap_or(Err(CommandError::NotFound))
        }
    }

    fn ok(stdout: &str) -> Result<CommandOutput, CommandError> {
        Ok(CommandOutput {
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
        })
    }

    fn fail(stderr: &str) -> Result<CommandOutput, CommandError> {
        Ok(CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: stderr.into(),
        })
    }

    const INFO: &str = "docker info --format {{.ServerVersion}}";

    #[test]
    fn docker_is_reported_in_three_distinct_layers() {
        assert_eq!(
            probe_docker(&Fake::new(vec![])),
            RuntimeAvailability::NotInstalled
        );

        let daemon_down = Fake::new(vec![
            ("docker --version", ok("Docker version 29.6.2")),
            (
                INFO,
                fail(
                    "Cannot connect to the Docker daemon at unix:///Users/x/.colima/default/docker.sock\nIs the docker daemon running?",
                ),
            ),
        ]);
        assert_eq!(
            probe_docker(&daemon_down),
            RuntimeAvailability::Unavailable {
                detail: "Cannot connect to the Docker daemon at unix:///Users/x/.colima/default/docker.sock".into()
            }
        );

        let up = Fake::new(vec![
            ("docker --version", ok("Docker version 29.6.2")),
            (INFO, ok("29.6.2\n")),
        ]);
        assert_eq!(
            probe_docker(&up),
            RuntimeAvailability::Available {
                detail: "docker 29.6.2".into()
            }
        );
    }

    #[test]
    fn a_silent_daemon_failure_still_gets_a_readable_reason_and_timeouts_are_unavailable() {
        let quiet = Fake::new(vec![
            ("docker --version", ok("Docker version 1")),
            (INFO, fail("")),
        ]);
        assert_eq!(
            probe_docker(&quiet),
            RuntimeAvailability::Unavailable {
                detail: "docker 守护进程不可用".into()
            }
        );
        let slow = Fake::new(vec![
            ("docker --version", ok("Docker version 1")),
            (INFO, Err(CommandError::Timeout)),
        ]);
        assert!(
            matches!(probe_docker(&slow), RuntimeAvailability::Unavailable { detail } if detail.contains("超时"))
        );
    }

    #[test]
    fn node_is_not_installed_available_or_broken() {
        assert_eq!(
            probe_node(&Fake::new(vec![])),
            RuntimeAvailability::NotInstalled
        );
        assert_eq!(
            probe_node(&Fake::new(vec![("node --version", ok("v24.14.0\n"))])),
            RuntimeAvailability::Available {
                detail: "node v24.14.0".into()
            }
        );
        assert_eq!(
            probe_node(&Fake::new(vec![(
                "node --version",
                fail("dyld: Library not loaded")
            )])),
            RuntimeAvailability::Unavailable {
                detail: "dyld: Library not loaded".into()
            }
        );
    }

    #[test]
    fn python_prefers_uv_then_python3_and_reads_old_pythons_stderr_version() {
        let both = Fake::new(vec![
            ("uv --version", ok("uv 0.9.1")),
            ("python3 --version", ok("Python 3.13.1")),
        ]);
        assert_eq!(
            probe_python(&both),
            RuntimeAvailability::Available {
                detail: "uv 0.9.1".into()
            }
        );

        let only_py = Fake::new(vec![("python3 --version", ok("Python 3.13.1\n"))]);
        assert_eq!(
            probe_python(&only_py),
            RuntimeAvailability::Available {
                detail: "Python 3.13.1".into()
            }
        );

        let old = Fake::new(vec![(
            "python3 --version",
            Ok(CommandOutput {
                success: true,
                stdout: String::new(),
                stderr: "Python 2.7.18\n".into(),
            }),
        )]);
        assert_eq!(
            probe_python(&old),
            RuntimeAvailability::Available {
                detail: "Python 2.7.18".into()
            }
        );

        assert_eq!(
            probe_python(&Fake::new(vec![])),
            RuntimeAvailability::NotInstalled
        );
        let broken_uv = Fake::new(vec![
            ("uv --version", fail("boom")),
            ("python3 --version", ok("Python 3.12.0")),
        ]);
        assert_eq!(
            probe_python(&broken_uv),
            RuntimeAvailability::Available {
                detail: "Python 3.12.0".into()
            },
            "uv 坏了就退到 python3"
        );
    }

    #[test]
    fn probe_all_covers_the_three_external_runtimes_in_a_fixed_order() {
        let names: Vec<_> = probe_all(&Fake::new(vec![]))
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, ["docker", "node", "python"]);
    }

    #[test]
    fn the_system_runner_reports_missing_programs_and_real_output() {
        let r = SystemRunner::default();
        assert_eq!(
            r.run("definitely-not-a-real-program-xyz", &[]),
            Err(CommandError::NotFound)
        );
        let out = r.run("sh", &["-c", "echo hello"]).unwrap();
        assert!(out.success);
        assert_eq!(out.stdout.trim(), "hello");
        let out = r.run("sh", &["-c", "echo oops >&2; exit 3"]).unwrap();
        assert!(!out.success);
        assert_eq!(out.stderr.trim(), "oops");
    }

    #[test]
    fn the_system_runner_kills_commands_that_exceed_the_timeout() {
        let r = SystemRunner {
            timeout: Duration::from_millis(150),
        };
        let started = Instant::now();
        assert_eq!(r.run("sleep", &["5"]), Err(CommandError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(3), "超时后立即返回");
    }
}
```

`$SCRATCH/a1/tests_manager.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::GatewayConfig;
    use crate::permissions::{Access, Enforcement, PermissionKey};
    use crate::plan::Approval;
    use crate::testutil::{Req, Resp, write_files};

    const HOST: Version = Version::new(0, 1, 0);

    fn manifest_toml(id: &str, version: &str, extra: &str) -> String {
        format!(
            r#"schema_version = 1
min_host_version = "0.1.0"
id = "{id}"
name = "{id} app"
version = "{version}"

[presentation]
entrypoint = "main"

[entrypoints.main]
type = "web"
path = "/"

[runtime]
kind = "static_web"
source = "web/"
{extra}"#
        )
    }

    /// 在 `dir` 下写一个静态应用(manifest.toml + web/index.html),返回来源。
    fn write_app(dir: &Path, id: &str, version: &str, extra: &str, body: &str) -> AppSource {
        write_files(
            dir,
            &[
                ("manifest.toml", &manifest_toml(id, version, extra)),
                ("web/index.html", body),
            ],
        );
        AppSource::LocalDir(dir.to_path_buf())
    }

    struct Rig {
        tmp: tempfile::TempDir,
        gateway: Arc<Gateway>,
        manager: AppManager,
    }

    async fn rig() -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let gateway = Arc::new(Gateway::start(GatewayConfig { port: 0 }).await.unwrap());
        let manager = AppManager::new(tmp.path().join("bytehost"), HOST, gateway.clone()).unwrap();
        Rig {
            tmp,
            gateway,
            manager,
        }
    }

    impl Rig {
        fn src_dir(&self, name: &str) -> PathBuf {
            self.tmp.path().join("src").join(name)
        }

        fn approve(&self, source: &AppSource) -> ApprovedInstallPlan {
            self.manager
                .install_plan(source, Provenance::Local, TrustLevel::Trusted)
                .unwrap()
                .approve(Approval {
                    approver: "test".into(),
                    approved_ms: 1,
                })
        }

        fn install(&self, source: &AppSource) -> Result<(), ManagerError> {
            self.manager.install(&self.approve(source), source, 100)
        }

        /// 带着有效 Cookie 取应用的某个路径。
        fn fetch(&self, id: &AppId, target: &str) -> Resp {
            let url = self.manager.launch_url(id);
            let token = url.split("bh_token=").nth(1).unwrap().to_string();
            let port = self.gateway.port();
            Req::get(port, &format!("{id}.localhost:{port}"), target)
                .cookie(&format!("bh_session={token}"))
                .send()
        }
    }

    fn id(s: &str) -> AppId {
        AppId::new(s).unwrap()
    }

    /// 安装失败之后,`apps/` 下不能留下 staging 目录,也不能凭空多出应用目录。
    fn assert_no_leftovers(rig: &Rig) {
        let apps = rig.manager.registry.paths().apps_dir();
        let names: Vec<String> = fs::read_dir(&apps)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().all(|n| !n.starts_with(".staging-")),
            "{names:?}"
        );
    }

    fn drain(rx: &mut broadcast::Receiver<AppEvent>) -> Vec<AppEvent> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_plan_describes_the_app_and_is_honest_about_static_web_enforcement() {
        let rig = rig().await;
        let src = write_app(
            &rig.src_dir("a"),
            "excalidraw",
            "0.17.0",
            "[permissions]\nclipboard = \"read_write\"\n",
            "<h1>x</h1>",
        );
        let plan = rig
            .manager
            .install_plan(&src, Provenance::ThirdParty, TrustLevel::Untrusted)
            .unwrap();
        assert_eq!(plan.app_id, id("excalidraw"));
        assert_eq!(plan.runtime_kind, "static_web");
        assert_eq!(plan.provenance, Provenance::ThirdParty);
        assert_eq!(plan.permission_diff.len(), 1);
        assert_eq!(plan.permission_diff[0].key, PermissionKey::Clipboard);
        let net = plan
            .enforcement
            .iter()
            .find(|e| e.key == PermissionKey::NetworkOutbound)
            .unwrap();
        assert_eq!(net.enforcement, Enforcement::Enforced);
        assert!(plan.will_run[0].contains("不执行任何命令"));
        assert_eq!(plan.manifest_digest.len(), 64);
        assert_eq!(plan.source_digest.len(), 64);
        assert!(
            rig.manager.list().unwrap().is_empty(),
            "出计划不会安装任何东西"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn install_start_serve_and_stop_round_trip_with_events() {
        let rig = rig().await;
        let mut rx = rig.manager.events();
        let src = write_app(
            &rig.src_dir("a"),
            "excalidraw",
            "0.17.0",
            "",
            "<h1>excalidraw</h1>",
        );
        rig.install(&src).unwrap();
        let app = id("excalidraw");

        let listed = rig.manager.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "excalidraw app");
        assert_eq!(listed[0].observed, ObservedState::Installed);
        assert_eq!(listed[0].desired, DesiredState::Stopped);
        assert_eq!(listed[0].url, None);
        assert_eq!(rig.fetch(&app, "/").status, 404, "没启动:站点没注册");

        let url = rig.manager.start(&app).unwrap();
        assert_eq!(
            url,
            format!("http://excalidraw.localhost:{}/", rig.gateway.port())
        );
        let r = rig.fetch(&app, "/");
        assert_eq!(r.status, 200);
        assert!(r.text().contains("excalidraw"));
        assert!(
            r.header("content-security-policy").is_some(),
            "默认出站网络为 none → 带 CSP"
        );
        let listed = rig.manager.list().unwrap();
        assert_eq!(
            (listed[0].observed.clone(), listed[0].url.clone()),
            (ObservedState::Running, Some(url.clone()))
        );
        assert_eq!(rig.manager.start(&app).unwrap(), url, "重复启动是幂等的");

        rig.manager.stop(&app).unwrap();
        assert_eq!(rig.fetch(&app, "/").status, 404);
        assert_eq!(
            rig.manager.list().unwrap()[0].observed,
            ObservedState::Stopped
        );
        rig.manager.stop(&app).unwrap();

        let events = drain(&mut rx);
        assert_eq!(
            events,
            vec![
                AppEvent::Installed { app: app.clone() },
                AppEvent::StateChanged {
                    app: app.clone(),
                    state: ObservedState::Running
                },
                AppEvent::EndpointChanged {
                    app: app.clone(),
                    url: Some(url)
                },
                AppEvent::StateChanged {
                    app: app.clone(),
                    state: ObservedState::Stopped
                },
                AppEvent::EndpointChanged {
                    app: app.clone(),
                    url: None
                },
            ]
        );
        assert!(
            !format!("{events:?}").contains("bh_token"),
            "事件里不能出现令牌"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn granting_outbound_network_removes_the_csp() {
        let rig = rig().await;
        let src = write_app(
            &rig.src_dir("a"),
            "online",
            "1.0.0",
            "[permissions.network]\noutbound = \"any\"\n",
            "hi",
        );
        rig.install(&src).unwrap();
        rig.manager.start(&id("online")).unwrap();
        let r = rig.fetch(&id("online"), "/");
        assert_eq!(r.status, 200);
        assert!(r.header("content-security-policy").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_source_or_manifest_changed_after_approval_is_refused_and_leaves_nothing_behind() {
        let rig = rig().await;
        let dir = rig.src_dir("a");
        let src = write_app(&dir, "excalidraw", "0.17.0", "", "<h1>good</h1>");
        let approved = rig.approve(&src);

        fs::write(dir.join("web/index.html"), "<h1>evil</h1>").unwrap();
        assert!(matches!(
            rig.manager.install(&approved, &src, 1),
            Err(ManagerError::Verify(VerifyError::SourceChanged))
        ));

        fs::write(dir.join("web/index.html"), "<h1>good</h1>").unwrap();
        fs::write(
            dir.join("manifest.toml"),
            manifest_toml(
                "excalidraw",
                "0.17.0",
                "[permissions]\nclipboard = \"read_write\"\n",
            ),
        )
        .unwrap();
        assert!(matches!(
            rig.manager.install(&approved, &src, 1),
            Err(ManagerError::Verify(VerifyError::ManifestChanged))
        ));

        assert!(rig.manager.list().unwrap().is_empty());
        assert_no_leftovers(&rig);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_forged_approved_payload_with_widened_permissions_is_refused() {
        let rig = rig().await;
        let src = write_app(&rig.src_dir("a"), "excalidraw", "0.17.0", "", "hi");
        let approved = rig.approve(&src);
        let mut json: serde_json::Value = serde_json::to_value(&approved).unwrap();
        json["plan"]["requested"]["clipboard"] = serde_json::json!("read_write");
        let forged: ApprovedInstallPlan = serde_json::from_value(json).unwrap();
        assert!(matches!(
            rig.manager.install(&forged, &src, 1),
            Err(ManagerError::Verify(VerifyError::PlanChanged))
        ));
        assert!(rig.manager.list().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn only_static_web_is_supported_in_phase_one() {
        let rig = rig().await;
        let dir = rig.src_dir("py");
        write_files(
            &dir,
            &[(
                "manifest.toml",
                &manifest_toml("pyapp", "1.0.0", "").replace(
                    "kind = \"static_web\"\nsource = \"web/\"",
                    "kind = \"python\"\ncommand = [\"python\", \"-m\", \"app\"]\n[runtime.http]\nport_env = \"PORT\"",
                ),
            )],
        );
        match rig.manager.install_plan(
            &AppSource::LocalDir(dir),
            Provenance::Local,
            TrustLevel::Trusted,
        ) {
            Err(ManagerError::UnsupportedRuntime(k)) => assert_eq!(k, "python"),
            other => panic!("{other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_package_with_a_symlink_is_refused() {
        let rig = rig().await;
        let dir = rig.src_dir("a");
        let src = write_app(&dir, "excalidraw", "0.17.0", "", "hi");
        std::os::unix::fs::symlink("/etc/hosts", dir.join("web/leak")).unwrap();
        assert!(matches!(
            rig.manager
                .install_plan(&src, Provenance::Local, TrustLevel::Trusted),
            Err(ManagerError::Io(_))
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_same_version_is_never_overwritten_and_an_upgrade_records_new_grants() {
        let rig = rig().await;
        let mut rx = rig.manager.events();
        let app = id("excalidraw");
        let v1 = write_app(
            &rig.src_dir("v1"),
            "excalidraw",
            "1.0.0",
            "",
            "<h1>one</h1>",
        );
        rig.install(&v1).unwrap();
        assert!(
            matches!(rig.install(&v1), Err(ManagerError::AlreadyInstalled(v)) if v == Version::new(1, 0, 0))
        );
        drain(&mut rx);

        let v2 = write_app(
            &rig.src_dir("v2"),
            "excalidraw",
            "1.1.0",
            "[permissions]\nclipboard = \"read\"\n",
            "<h1>two</h1>",
        );
        let plan = rig
            .manager
            .install_plan(&v2, Provenance::Local, TrustLevel::Trusted)
            .unwrap();
        assert_eq!(plan.upgrading_from, Some(Version::new(1, 0, 0)));
        assert_eq!(plan.permission_diff.len(), 1, "升级差异是相对当前授予的");
        rig.install(&v2).unwrap();

        let events = drain(&mut rx);
        assert!(matches!(&events[0], AppEvent::Installed { .. }));
        assert!(
            matches!(&events[1], AppEvent::ManifestChanged { permission_changes, .. } if permission_changes.len() == 1)
        );
        let record = rig.manager.registry.load(&app).unwrap().unwrap();
        assert_eq!(record.current_version, Version::new(1, 1, 0));
        assert_eq!(record.versions.len(), 2);
        assert_eq!(record.grants.clipboard, Access::Read);
        let pkg = rig.manager.registry.paths();
        assert!(
            pkg.package_dir(&app, &Version::new(1, 0, 0)).is_dir(),
            "旧版本目录保留"
        );
        rig.manager.start(&app).unwrap();
        assert!(
            rig.fetch(&app, "/").text().contains("two"),
            "启动的是当前版本"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upgrading_a_running_app_is_refused_until_it_is_stopped() {
        let rig = rig().await;
        let app = id("excalidraw");
        rig.install(&write_app(
            &rig.src_dir("v1"),
            "excalidraw",
            "1.0.0",
            "",
            "one",
        ))
        .unwrap();
        rig.manager.start(&app).unwrap();
        let v2 = write_app(&rig.src_dir("v2"), "excalidraw", "1.1.0", "", "two");
        assert!(matches!(rig.install(&v2), Err(ManagerError::Busy(_))));
        rig.manager.stop(&app).unwrap();
        rig.install(&v2).unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_package_without_its_site_directory_fails_to_start_cleanly_and_stop_resets_it() {
        let rig = rig().await;
        let dir = rig.src_dir("a");
        write_files(
            &dir,
            &[("manifest.toml", &manifest_toml("hollow", "1.0.0", ""))],
        );
        let src = AppSource::LocalDir(dir);
        rig.install(&src).unwrap();
        let app = id("hollow");
        assert!(matches!(
            rig.manager.start(&app),
            Err(ManagerError::MissingSource(_))
        ));
        assert!(matches!(
            rig.manager.list().unwrap()[0].observed,
            ObservedState::Failed {
                retryable: false,
                ..
            }
        ));
        assert!(
            matches!(rig.manager.start(&app), Err(ManagerError::BadState { .. })),
            "不可重试的失败不自动重来"
        );
        rig.manager.stop(&app).unwrap();
        assert_eq!(
            rig.manager.list().unwrap()[0].observed,
            ObservedState::Stopped
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn uninstalling_the_program_stops_serving_and_keeps_user_data() {
        let rig = rig().await;
        let mut rx = rig.manager.events();
        let app = id("excalidraw");
        rig.install(&write_app(
            &rig.src_dir("a"),
            "excalidraw",
            "1.0.0",
            "",
            "hi",
        ))
        .unwrap();
        rig.manager.start(&app).unwrap();
        let data = rig.manager.registry.paths().data_dir(&app);
        fs::write(data.join("drawing.json"), "{}").unwrap();
        drain(&mut rx);

        rig.manager.uninstall(&app, UninstallMode::Program).unwrap();
        assert!(rig.manager.list().unwrap().is_empty());
        assert!(!rig.gateway.has_site(&app));
        assert!(data.join("drawing.json").exists());
        let events = drain(&mut rx);
        assert!(events.contains(&AppEvent::StateChanged {
            app: app.clone(),
            state: ObservedState::NotInstalled
        }));

        rig.manager.uninstall(&app, UninstallMode::Program).unwrap();
        rig.manager
            .uninstall(&app, UninstallMode::ProgramAndData)
            .unwrap();
        assert!(!rig.manager.registry.paths().app_dir(&app).exists());
        rig.manager
            .uninstall(&id("never-installed"), UninstallMode::ProgramAndData)
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn after_a_supervisor_restart_reconcile_restarts_apps_that_wanted_to_run() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("bytehost");
        let (run, idle) = (id("runner"), id("idler"));
        {
            let gw = Arc::new(Gateway::start(GatewayConfig { port: 0 }).await.unwrap());
            let m = AppManager::new(&root, HOST, gw.clone()).unwrap();
            for (app, dir) in [(&run, "runner"), (&idle, "idler")] {
                let src = write_app(
                    &tmp.path().join("src").join(dir),
                    app.as_str(),
                    "1.0.0",
                    "",
                    "hello",
                );
                let plan = m
                    .install_plan(&src, Provenance::Local, TrustLevel::Trusted)
                    .unwrap();
                m.install(
                    &plan.approve(Approval {
                        approver: "t".into(),
                        approved_ms: 1,
                    }),
                    &src,
                    1,
                )
                .unwrap();
            }
            m.start(&run).unwrap();
            // "崩溃":不做任何清理,直接丢掉 gateway 与 manager
            drop(m);
            Arc::try_unwrap(gw).ok().unwrap().shutdown().await;
        }
        let gw2 = Arc::new(Gateway::start(GatewayConfig { port: 0 }).await.unwrap());
        let m2 = AppManager::new(&root, HOST, gw2.clone()).unwrap();
        let before = m2.list().unwrap();
        let stale = before.iter().find(|a| a.id == run).unwrap();
        assert_eq!(
            stale.observed,
            ObservedState::Running,
            "磁盘上是崩溃前的陈旧观察态"
        );
        assert!(!gw2.has_site(&run), "但新的 gateway 里没有站点");

        assert!(m2.reconcile().is_empty());
        assert!(gw2.has_site(&run), "desired=Running 的应用被重新启动");
        assert!(!gw2.has_site(&idle), "desired=Stopped 的保持停止");
        let after = m2.list().unwrap();
        assert_eq!(
            after.iter().find(|a| a.id == run).unwrap().observed,
            ObservedState::Running
        );
        assert_eq!(
            after.iter().find(|a| a.id == idle).unwrap().observed,
            ObservedState::Installed
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reconcile_reports_a_broken_app_without_blocking_the_others() {
        let rig = rig().await;
        let (good, bad) = (id("good"), id("bad"));
        rig.install(&write_app(&rig.src_dir("good"), "good", "1.0.0", "", "ok"))
            .unwrap();
        rig.install(&write_app(&rig.src_dir("bad"), "bad", "1.0.0", "", "ok"))
            .unwrap();
        rig.manager.start(&good).unwrap();
        rig.manager.start(&bad).unwrap();
        // 模拟崩溃后:坏应用的站点目录被删,两个应用都停在"想运行"
        rig.gateway.remove_site(&good);
        rig.gateway.remove_site(&bad);
        let pkg = rig
            .manager
            .registry
            .paths()
            .package_dir(&bad, &Version::new(1, 0, 0));
        fs::remove_dir_all(pkg.join("web")).unwrap();

        let failures = rig.manager.reconcile();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, bad);
        assert!(matches!(failures[0].1, ManagerError::MissingSource(_)));
        assert!(rig.gateway.has_site(&good));
        assert!(!rig.gateway.has_site(&bad));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn listing_falls_back_to_the_id_when_the_manifest_cannot_be_read() {
        let rig = rig().await;
        let app = id("excalidraw");
        rig.install(&write_app(
            &rig.src_dir("a"),
            "excalidraw",
            "1.0.0",
            "",
            "hi",
        ))
        .unwrap();
        fs::write(
            rig.manager.registry.paths().manifest_path(&app),
            "not toml {{{",
        )
        .unwrap();
        assert_eq!(rig.manager.list().unwrap()[0].name, "excalidraw");
    }

    /// 安装的每个失败出口(摘要对不上、同版本、应用在运行)都不能留下 staging;而且保存下来的 manifest
    /// 是 staging 副本里被 verify 过的那份。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn every_failed_install_cleans_up_and_the_saved_manifest_is_the_staged_one() {
        let rig = rig().await;
        let dir = rig.src_dir("a");
        let src = write_app(&dir, "excalidraw", "1.0.0", "", "one");
        rig.install(&src).unwrap();
        let saved = fs::read_to_string(
            rig.manager
                .registry
                .paths()
                .manifest_path(&id("excalidraw")),
        )
        .unwrap();
        assert_eq!(
            saved,
            fs::read_to_string(dir.join("manifest.toml")).unwrap()
        );

        assert!(matches!(
            rig.install(&src),
            Err(ManagerError::AlreadyInstalled(_))
        ));
        assert_no_leftovers(&rig);

        rig.manager.start(&id("excalidraw")).unwrap();
        let v2 = write_app(&rig.src_dir("v2"), "excalidraw", "1.1.0", "", "two");
        assert!(matches!(rig.install(&v2), Err(ManagerError::Busy(_))));
        assert_no_leftovers(&rig);

        let unsupported = rig.src_dir("py");
        write_files(
            &unsupported,
            &[(
                "manifest.toml",
                &manifest_toml("py", "1.0.0", "").replace(
                    "kind = \"static_web\"\nsource = \"web/\"",
                    "kind = \"python\"\ncommand = [\"p\"]\n[runtime.http]\nport_env = \"PORT\"",
                ),
            )],
        );
        assert!(
            rig.manager
                .install(&rig.approve(&src), &AppSource::LocalDir(unsupported), 1)
                .is_err()
        );
        assert_no_leftovers(&rig);
    }
}
```

`$SCRATCH/a1/Cargo.toml`(完整的新 crate 清单):

```toml
[package]
name = "bytehost-apps"
version = "0.1.0"
edition.workspace = true
license.workspace = true
publish = false
description = "bytehost 应用宿主的无界面部分:应用模型、安装计划、生命周期状态、注册表。不依赖任何 dozer crate。"

[features]
# 默认 feature 为空:只有类型与纯逻辑,依赖仅 serde/serde_json——`dozer-core` 依赖本 crate 时,
# `dozer-hook`/`dozer-mcp` 的依赖闭包不会因此增加新 crate。
# 从 TOML 文本解析 manifest(dozerd 与安装流程用)。
manifest-toml = ["dep:toml"]
# 摘要计算(manifest 摘要、源码目录摘要、每应用数据存储标识)。
digest = ["dep:sha2"]
# 服务端部分:`AppManager`、runtime adapter、gateway(本机 HTTP 服务)。只有 dozerd 打开;
# 隐含 `digest` 与 `manifest-toml`。
server = [
    "digest",
    "manifest-toml",
    "dep:tokio",
    "dep:hyper",
    "dep:hyper-util",
    "dep:http-body-util",
    "dep:bytes",
    "dep:uuid",
]

[dependencies]
serde.workspace = true
serde_json.workspace = true
toml = { version = "0.8", optional = true }
sha2 = { version = "0.10", optional = true }
tokio = { workspace = true, optional = true }
hyper = { version = "1", optional = true, features = ["server", "http1"] }
hyper-util = { version = "0.1", optional = true, features = ["tokio"] }
http-body-util = { version = "0.1", optional = true }
bytes = { version = "1", optional = true }
uuid = { workspace = true, optional = true }

[dev-dependencies]
tempfile = "3"
# 测试里用 tokio 的 `#[tokio::test]`;`tokio` 本身在 `server` feature 下才是依赖
tokio = { workspace = true }
```

`$SCRATCH/a1/lib.rs`:

```rust
//! bytehost 应用宿主的无界面部分:应用模型、安装计划、生命周期状态、注册表。
//!
//! 设计见 `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`。本 crate **不依赖任何 dozer crate**
//! (门禁:`scripts/check-bytehost-apps-deps.sh`),也不含界面/wry/iced。
//!
//! Cargo features(默认全空,只有类型与纯逻辑,依赖仅 serde/serde_json):
//! - `manifest-toml`:`Manifest::from_toml`
//! - `digest`:`digest` 模块(SHA-256、源码目录摘要、数据存储标识)
//! - `server`:`gateway`、`runtime`、`manager`、`port`(只有 dozerd 打开;隐含 `digest` 与 `manifest-toml`)

#[cfg(feature = "digest")]
pub mod digest;
pub mod event;
#[cfg(feature = "server")]
pub mod gateway;
pub mod id;
#[cfg(feature = "server")]
pub mod manager;
pub mod manifest;
pub mod permissions;
pub mod plan;
#[cfg(feature = "server")]
pub mod port;
pub mod registry;
#[cfg(feature = "server")]
pub mod runtime;
pub mod state;
#[cfg(all(test, feature = "server"))]
mod testutil;
```

`$SCRATCH/a1/testutil.rs`:

```rust
//! 测试辅助(只在 `cargo test --features server` 下编译):一个最小的原始 TCP HTTP/1.1 客户端,
//! 不依赖任何 HTTP 客户端库,这样测试能精确控制 `Host`、`Cookie` 等头,模拟"伪造 Host"之类的请求。

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;

#[derive(Debug)]
pub(crate) struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub(crate) struct Req<'a> {
    pub port: u16,
    pub method: &'a str,
    pub host: Option<&'a str>,
    pub target: &'a str,
    pub cookie: Option<&'a str>,
}

impl<'a> Req<'a> {
    pub fn get(port: u16, host: &'a str, target: &'a str) -> Self {
        Self {
            port,
            method: "GET",
            host: Some(host),
            target,
            cookie: None,
        }
    }

    pub fn cookie(mut self, cookie: &'a str) -> Self {
        self.cookie = Some(cookie);
        self
    }

    pub fn method(mut self, method: &'a str) -> Self {
        self.method = method;
        self
    }

    pub fn send(&self) -> Resp {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).expect("连上 gateway");
        let mut raw = format!("{} {} HTTP/1.1\r\n", self.method, self.target);
        if let Some(h) = self.host {
            raw.push_str(&format!("Host: {h}\r\n"));
        }
        if let Some(c) = self.cookie {
            raw.push_str(&format!("Cookie: {c}\r\n"));
        }
        raw.push_str("Connection: close\r\n\r\n");
        stream.write_all(raw.as_bytes()).expect("发请求");
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).expect("读响应");
        parse(&buf)
    }
}

fn parse(buf: &[u8]) -> Resp {
    let split = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("响应有头");
    let head = String::from_utf8_lossy(&buf[..split]).into_owned();
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .expect("状态行");
    let headers = lines
        .filter_map(|l| {
            l.split_once(':')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        })
        .collect();
    Resp {
        status,
        headers,
        body: buf[split + 4..].to_vec(),
    }
}

/// 在 `dir` 下按 (相对路径, 内容) 写出文件。
pub(crate) fn write_files(dir: &Path, files: &[(&str, &str)]) {
    for (rel, content) in files {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }
}
```

骨架脚本 `$SCRATCH/a1_skeleton.py`(带引号 heredoc),运行并确认编译失败:

```python
#!/usr/bin/env python3
"""A1 一次性脚本(步骤 3):改 Cargo.toml/lib.rs、写测试辅助 `testutil.rs`,并为每个新模块建一个**只含测试**的文件,
这样编译会因为缺类型而失败(RED)。`SP` 是 tests_*.rs 等所在目录。**不提交。**"""
import os, shutil

SP = os.environ["SP"]
CRATE = "crates/bytehost-apps"
FILES = {
    "gateway_mod": "gateway/mod.rs",
    "gateway_static_files": "gateway/static_files.rs",
    "port": "port.rs",
    "runtime_mod": "runtime/mod.rs",
    "runtime_probe": "runtime/probe.rs",
    "manager": "manager.rs",
}
shutil.copy(os.path.join(SP, "Cargo.toml"), CRATE + "/Cargo.toml")
shutil.copy(os.path.join(SP, "lib.rs"), CRATE + "/src/lib.rs")
shutil.copy(os.path.join(SP, "testutil.rs"), CRATE + "/src/testutil.rs")
for name, rel in FILES.items():
    dst = f"{CRATE}/src/{rel}"
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copy(os.path.join(SP, f"tests_{name}.rs"), dst)
print("ok")
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a1 && export PYTHONDONTWRITEBYTECODE=1
SP=$SCRATCH/a1 python3 $SCRATCH/a1_skeleton.py
cargo test -p bytehost-apps --features server --no-run 2>&1 | grep -E "^error" | sort | uniq -c | head -8
```
Expected: 脚本输出 `ok`;编译失败(`could not compile bytehost-apps (lib test)`),报错是 `cannot find …`(如 `GatewayConfig`、`Site`、`State`、`cookie_value`、`AppManager`、`pick_port`、`probe_docker`……)——RED,测试引用的实现都还不存在。

- [ ] **Step 4: 写实现源文件**

把下面 6 段实现写成 `$SCRATCH/a1/impl_<模块>.rs`:

`$SCRATCH/a1/impl_gateway_static_files.rs`:

```rust
//! 静态文件服务:URL 路径 → 应用包里的文件。**路径解析是安全边界**——任何一种写法都不能走出站点根目录。

use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Resolved {
    File(PathBuf),
    NotFound,
    /// 试图走出根目录(`..`、符号链接逃逸)。
    Forbidden,
    /// 编码非法、含 NUL 或反斜杠。
    BadRequest,
}

/// 百分号解码。非法序列(`%` 后不是两位十六进制)或解出来不是 UTF-8 返回 `None`。
pub(crate) fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = hex_val(*bytes.get(i + 1)?)?;
            let lo = hex_val(*bytes.get(i + 2)?)?;
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// 把 URL 路径(不含查询串)解析成 `root` 之下的文件。目录与 `/` 落到 `index.html`。
pub(crate) fn resolve(root: &Path, url_path: &str) -> Resolved {
    let Some(decoded) = percent_decode(url_path) else {
        return Resolved::BadRequest;
    };
    if decoded.contains('\0') || decoded.contains('\\') {
        return Resolved::BadRequest;
    }
    let mut candidate = root.to_path_buf();
    for seg in decoded.split('/') {
        match seg {
            "" | "." => {}
            ".." => return Resolved::Forbidden,
            s => candidate.push(s),
        }
    }
    if candidate.is_dir() {
        candidate.push("index.html");
    }
    let (Ok(real), Ok(real_root)) = (candidate.canonicalize(), root.canonicalize()) else {
        return Resolved::NotFound;
    };
    if !real.starts_with(&real_root) {
        return Resolved::Forbidden;
    }
    if real.is_file() {
        Resolved::File(real)
    } else {
        Resolved::NotFound
    }
}

pub(crate) fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}
```

`$SCRATCH/a1/impl_gateway_mod.rs`:

```rust
//! gateway:本机 HTTP 服务,**一个固定端口服务所有应用**,按 `Host` 头路由到各应用的站点
//! (`http://<app-id>.localhost:<端口>/`)。设计依据与实测见 `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`
//! §5 与 `spike/origin-gateway/README.md`。
//!
//! 安全顺序(每一步失败就停,不泄露后面步骤的信息):
//! 1. **Host 校验**:必须恰好是 `<合法 app id>.localhost:<本 gateway 的端口>`,否则 421(防 DNS rebinding);
//! 2. **会话令牌**:请求要带 gateway 启动时生成的令牌——首次导航用 `?bh_token=…`,gateway 设 host-only 的
//!    `HttpOnly; SameSite=Strict` Cookie 并 302 到去掉令牌的地址;之后靠 Cookie。没有令牌一律 403
//!    (连"这个应用存不存在"都不告诉本机上的其他网页);
//! 3. **站点查找**:没注册的应用 404;
//! 4. **方法**:只允许 GET/HEAD;
//! 5. **路径解析**(`static_files::resolve`):任何写法都不能走出站点根目录。

mod static_files;

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{self, HeaderValue};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::Notify;

use crate::id::AppId;
use crate::permissions::{Outbound, Permissions};

use static_files::{Resolved, mime_for, resolve};

const TOKEN_PARAM: &str = "bh_token";
const COOKIE_NAME: &str = "bh_session";

#[derive(Debug)]
pub enum GatewayError {
    /// 配置的端口已被占用。**不会静默换端口**——换端口会让所有应用丢失本地存储(origin 含端口)。
    PortInUse(u16),
    Io(std::io::Error),
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PortInUse(p) => write!(
                f,
                "gateway 端口 {p} 已被占用(不会自动换端口:换端口会让所有应用丢失本地数据)"
            ),
            Self::Io(e) => write!(f, "gateway I/O 错误: {e}"),
        }
    }
}

impl std::error::Error for GatewayError {}

#[derive(Debug, Clone, Copy)]
pub struct GatewayConfig {
    /// 监听端口(127.0.0.1)。`0` 让系统分配——只给测试用;生产用 `port::load_or_choose_port` 的值。
    pub port: u16,
}

/// 一个静态站点:根目录与(可选的)`Content-Security-Policy`。
#[derive(Debug, Clone)]
struct Site {
    root: PathBuf,
    csp: Option<String>,
}

struct State {
    port: u16,
    token: String,
    sites: RwLock<HashMap<String, Site>>,
}

/// 运行中的 gateway。`Drop` 不会停服务,要停请 [`Gateway::shutdown`]。
pub struct Gateway {
    state: Arc<State>,
    shutdown: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

/// 按**已授予**的权限算 CSP:出站网络为 `none` 时,用 CSP 真正禁止页面向外发请求(`Enforced`);
/// 为 `any` 时不加 CSP。
pub fn csp_for(grants: &Permissions) -> Option<String> {
    match grants.network.outbound {
        Outbound::Any => None,
        Outbound::None => Some(
            "default-src 'self' data: blob:; script-src 'self' 'wasm-unsafe-eval'; \
             style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; \
             connect-src 'self'; base-uri 'self'; form-action 'self'"
                .to_string(),
        ),
    }
}

impl Gateway {
    pub async fn start(config: GatewayConfig) -> Result<Self, GatewayError> {
        let listener = TcpListener::bind(("127.0.0.1", config.port))
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AddrInUse {
                    GatewayError::PortInUse(config.port)
                } else {
                    GatewayError::Io(e)
                }
            })?;
        let port = listener.local_addr().map_err(GatewayError::Io)?.port();
        let state = Arc::new(State {
            port,
            token: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            sites: RwLock::new(HashMap::new()),
        });
        let shutdown = Arc::new(Notify::new());
        let task = tokio::spawn(accept_loop(listener, state.clone(), shutdown.clone()));
        Ok(Self {
            state,
            shutdown,
            task,
        })
    }

    pub fn port(&self) -> u16 {
        self.state.port
    }

    /// 注册(或替换)一个应用的静态站点。
    pub fn add_site(&self, id: &AppId, root: PathBuf, csp: Option<String>) {
        self.state
            .sites
            .write()
            .expect("sites 锁")
            .insert(id.as_str().to_string(), Site { root, csp });
    }

    /// 注销站点;返回之前是否注册过。
    pub fn remove_site(&self, id: &AppId) -> bool {
        self.state
            .sites
            .write()
            .expect("sites 锁")
            .remove(id.as_str())
            .is_some()
    }

    pub fn has_site(&self, id: &AppId) -> bool {
        self.state
            .sites
            .read()
            .expect("sites 锁")
            .contains_key(id.as_str())
    }

    /// 不含令牌的站点地址(可以放进事件/日志)。
    pub fn site_url(&self, id: &AppId) -> String {
        format!("http://{id}.localhost:{}/", self.state.port)
    }

    /// 首次导航用的地址:带一次性换 Cookie 的令牌。**这是秘密,不要写进日志或广播事件。**
    pub fn launch_url(&self, id: &AppId) -> String {
        format!("{}?{TOKEN_PARAM}={}", self.site_url(id), self.state.token)
    }

    pub async fn shutdown(self) {
        // notify_one 会存一个许可:即使 accept 循环还没跑到 `notified()`,稍后也能收到(notify_waiters 会丢)
        self.shutdown.notify_one();
        let _ = self.task.await;
    }
}

async fn accept_loop(listener: TcpListener, state: Arc<State>, shutdown: Arc<Notify>) {
    loop {
        tokio::select! {
            _ = shutdown.notified() => break,
            accepted = listener.accept() => {
                let Ok((stream, _addr)) = accepted else { continue };
                let state = state.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |req: Request<Incoming>| {
                        let state = state.clone();
                        async move {
                            let method = req.method().clone();
                            let host = header_str(&req, header::HOST);
                            let cookie = header_str(&req, header::COOKIE);
                            let target = req
                                .uri()
                                .path_and_query()
                                .map(|p| p.as_str().to_string())
                                .unwrap_or_else(|| "/".to_string());
                            let reply = tokio::task::spawn_blocking(move || {
                                respond(&state, &method, host.as_deref(), &target, cookie.as_deref())
                            })
                            .await
                            .unwrap_or_else(|_| plain(StatusCode::INTERNAL_SERVER_ERROR, "internal error"));
                            Ok::<_, Infallible>(reply)
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        }
    }
}

fn header_str(req: &Request<Incoming>, name: header::HeaderName) -> Option<String> {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

type Reply = Response<Full<Bytes>>;

fn plain(status: StatusCode, body: &'static str) -> Reply {
    let mut r = Response::new(Full::new(Bytes::from_static(body.as_bytes())));
    *r.status_mut() = status;
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    harden(r.headers_mut());
    r
}

fn harden(h: &mut header::HeaderMap) {
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
}

/// `Host` 头必须恰好是 `<合法 app id>.localhost:<port>`;返回 app id。
fn app_from_host(host: Option<&str>, port: u16) -> Option<AppId> {
    let (name, host_port) = host?.rsplit_once(':')?;
    if host_port.parse::<u16>().ok()? != port {
        return None;
    }
    AppId::new(name.strip_suffix(".localhost")?).ok()
}

fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').find_map(|part| {
        let (k, v) = part.trim().split_once('=')?;
        (k == name).then_some(v)
    })
}

/// 常量时间比较(不因第一个不同字节提前返回)。
fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// 从查询串里取出令牌,返回 (令牌, 去掉令牌后的查询串)。
fn split_token(query: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(q) = query else { return (None, None) };
    let mut token = None;
    let mut rest = Vec::new();
    for pair in q.split('&') {
        match pair.split_once('=') {
            Some((k, v)) if k == TOKEN_PARAM => token = Some(v.to_string()),
            _ if pair.is_empty() => {}
            _ => rest.push(pair),
        }
    }
    (token, (!rest.is_empty()).then(|| rest.join("&")))
}

/// 处理一个请求。纯同步、不碰网络,便于直接测试;`target` 是请求行里的 path+query。
fn respond(
    state: &State,
    method: &Method,
    host: Option<&str>,
    target: &str,
    cookie: Option<&str>,
) -> Reply {
    // 1. Host
    let Some(app) = app_from_host(host, state.port) else {
        return plain(StatusCode::MISDIRECTED_REQUEST, "misdirected request");
    };
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (target, None),
    };
    // 2. 令牌
    let (query_token, rest_query) = split_token(query);
    if let Some(t) = &query_token {
        if !ct_eq(t, &state.token) {
            return plain(StatusCode::FORBIDDEN, "forbidden");
        }
        let location = match &rest_query {
            Some(q) => format!("{path}?{q}"),
            None => path.to_string(),
        };
        let mut r = Response::new(Full::new(Bytes::new()));
        *r.status_mut() = StatusCode::FOUND;
        r.headers_mut().insert(
            header::LOCATION,
            HeaderValue::from_str(&location).unwrap_or_else(|_| HeaderValue::from_static("/")),
        );
        r.headers_mut().insert(
            header::SET_COOKIE,
            HeaderValue::from_str(&format!(
                "{COOKIE_NAME}={}; HttpOnly; SameSite=Strict; Path=/",
                state.token
            ))
            .expect("令牌是十六进制"),
        );
        harden(r.headers_mut());
        return r;
    }
    let authed = cookie
        .and_then(|c| cookie_value(c, COOKIE_NAME))
        .is_some_and(|v| ct_eq(v, &state.token));
    if !authed {
        return plain(StatusCode::FORBIDDEN, "forbidden");
    }
    // 3. 站点
    let Some(site) = state
        .sites
        .read()
        .expect("sites 锁")
        .get(app.as_str())
        .cloned()
    else {
        return plain(StatusCode::NOT_FOUND, "no such app");
    };
    // 4. 方法
    if method != Method::GET && method != Method::HEAD {
        let mut r = plain(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
        r.headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return r;
    }
    // 5. 路径
    match resolve(&site.root, path) {
        Resolved::File(file) => serve_file(&file, &site, method == Method::HEAD),
        Resolved::NotFound => plain(StatusCode::NOT_FOUND, "not found"),
        Resolved::Forbidden => plain(StatusCode::FORBIDDEN, "forbidden"),
        Resolved::BadRequest => plain(StatusCode::BAD_REQUEST, "bad request"),
    }
}

fn serve_file(file: &Path, site: &Site, head_only: bool) -> Reply {
    let Ok(bytes) = std::fs::read(file) else {
        return plain(StatusCode::NOT_FOUND, "not found");
    };
    let len = bytes.len();
    let mut r = Response::new(Full::new(if head_only {
        Bytes::new()
    } else {
        Bytes::from(bytes)
    }));
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(mime_for(file)),
    );
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    harden(h);
    if let Some(csp) = &site.csp
        && let Ok(v) = HeaderValue::from_str(csp)
    {
        h.insert(header::CONTENT_SECURITY_POLICY, v);
    }
    r
}
```

`$SCRATCH/a1/impl_port.rs`:

```rust
//! gateway 固定端口:首次启动从高端口段随机选一个并持久化(`<root>/gateway.json`),跨重启不变。
//! **端口一变,所有应用的本地存储(origin 含端口)都会丢**——所以坏文件、越界端口一律报错,
//! 绝不"顺手重新选一个";被占用的处理见 `GatewayError::PortInUse`(同样不静默换)。

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// 随机选取的范围(IANA 动态/私有端口段,避开 3000/8080 之类的常用开发端口)。
pub const PORT_MIN: u16 = 49152;
pub const PORT_MAX: u16 = 65000;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GatewaySettings {
    port: u16,
}

/// 在 `PORT_MIN..=PORT_MAX` 里随机选一个。
pub fn pick_port() -> u16 {
    let bytes = *uuid::Uuid::new_v4().as_bytes();
    let n = u16::from_le_bytes([bytes[0], bytes[1]]);
    PORT_MIN + n % (PORT_MAX - PORT_MIN + 1)
}

/// 读 `<root>/gateway.json` 里持久化的端口;文件不存在就选一个、原子写入、返回。
pub fn load_or_choose_port(root: &Path) -> io::Result<u16> {
    let path = root.join("gateway.json");
    match fs::read_to_string(&path) {
        Ok(text) => {
            let settings: GatewaySettings = serde_json::from_str(&text).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: {e}", path.display()),
                )
            })?;
            if settings.port < 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{}: 端口 {} 不在允许范围(>= 1024)",
                        path.display(),
                        settings.port
                    ),
                ));
            }
            Ok(settings.port)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let port = pick_port();
            fs::create_dir_all(root)?;
            let json = serde_json::to_string_pretty(&GatewaySettings { port })
                .map_err(io::Error::other)?;
            let tmp = path.with_extension("tmp");
            fs::write(&tmp, json)?;
            fs::rename(&tmp, &path)?;
            Ok(port)
        }
        Err(e) => Err(e),
    }
}
```

`$SCRATCH/a1/impl_runtime_mod.rs`:

```rust
//! runtime adapter 的公共部分:各运行时的**可用性探测**(`probe`)与**强制等级表**(`enforcement_for`)。
//! 一期只有 `static_web` 真正实现;Node/Python/容器只定义探测与强制等级,供安装计划和 Settings 展示。

mod probe;

pub use probe::{
    CommandError, CommandOutput, CommandRunner, RuntimeAvailability, SystemRunner, probe_all,
    probe_docker, probe_node, probe_python,
};

use crate::manifest::Runtime;
use crate::permissions::{Enforcement, PermissionKey};
use crate::plan::EnforcementEntry;

/// 这个 runtime 上每条权限的强制等级(**诚实**:做不到就标 `Advisory`/`Unsupported`)。
///
/// - `static_web`:出站网络由 gateway 的 CSP 真正禁止(`Enforced`);静态页没有服务端数据目录
///   (`Unsupported`);剪贴板/下载/弹窗取决于承载它的 WebView(产品层),gateway 管不到(`Advisory`)。
/// - `node`/`python`:macOS 上没有轻量进程沙箱,全部 `Advisory`——只是声明,不隔离。
/// - `container`:挂载(`filesystem`)可由容器真正限制(`Enforced`);出站网络要额外的网络/代理配置,
///   一期不做(`Advisory`);其余同样取决于 WebView。
pub fn enforcement_for(runtime: &Runtime) -> Vec<EnforcementEntry> {
    use Enforcement::*;
    use PermissionKey::*;
    let table: [(PermissionKey, Enforcement); 5] = match runtime {
        Runtime::StaticWeb { .. } => [
            (NetworkOutbound, Enforced),
            (FilesystemData, Unsupported),
            (Clipboard, Advisory),
            (Downloads, Advisory),
            (Popups, Advisory),
        ],
        Runtime::Node { .. } | Runtime::Python { .. } => [
            (NetworkOutbound, Advisory),
            (FilesystemData, Advisory),
            (Clipboard, Advisory),
            (Downloads, Advisory),
            (Popups, Advisory),
        ],
        Runtime::Container { .. } => [
            (NetworkOutbound, Advisory),
            (FilesystemData, Enforced),
            (Clipboard, Advisory),
            (Downloads, Advisory),
            (Popups, Advisory),
        ],
    };
    table
        .into_iter()
        .map(|(key, enforcement)| EnforcementEntry { key, enforcement })
        .collect()
}
```

`$SCRATCH/a1/impl_runtime_probe.rs`:

```rust
//! 运行时可用性探测:分层报告"没装 / 装了但当前不可用 / 可用",产品据此给出不同的提示
//! (没装 → 去 Settings 安装;装了没启动 → 启动它,如 Colima)。外部命令经 [`CommandRunner`] 执行,
//! 测试里换成假的,不依赖本机装了什么。

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeAvailability {
    /// 可用;`detail` 是版本之类的人类可读信息。
    Available {
        detail: String,
    },
    /// 装了但当前用不了(如 Docker 守护进程没启动)。
    Unavailable {
        detail: String,
    },
    NotInstalled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// 找不到这个可执行文件。
    NotFound,
    Timeout,
    Failed(String),
}

pub trait CommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, CommandError>;
}

/// 真正执行外部命令,带超时(超时就杀掉子进程)。
pub struct SystemRunner {
    pub timeout: Duration,
}

impl Default for SystemRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
        }
    }
}

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, CommandError> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    CommandError::NotFound
                } else {
                    CommandError::Failed(e.to_string())
                }
            })?;
        let deadline = Instant::now() + self.timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let read = |pipe: Option<&mut dyn Read>| {
                        let mut s = String::new();
                        if let Some(p) = pipe {
                            let _ = p.read_to_string(&mut s);
                        }
                        s
                    };
                    let stdout = read(child.stdout.as_mut().map(|p| p as &mut dyn Read));
                    let stderr = read(child.stderr.as_mut().map(|p| p as &mut dyn Read));
                    return Ok(CommandOutput {
                        success: status.success(),
                        stdout,
                        stderr,
                    });
                }
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(CommandError::Timeout);
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(CommandError::Failed(e.to_string())),
            }
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

fn unavailable(
    program: &str,
    err: CommandError,
    out: Option<CommandOutput>,
) -> RuntimeAvailability {
    let detail = match (err, out) {
        (CommandError::Timeout, _) => format!("{program} 执行超时"),
        (CommandError::Failed(e), _) => e,
        (CommandError::NotFound, _) => unreachable!("NotFound 在调用处单独处理"),
    };
    RuntimeAvailability::Unavailable { detail }
}

/// Docker:先看 `docker` 命令在不在,再用 `docker info` 看守护进程连得上连不上(Colima 没启动时命令在、守护进程不可用)。
pub fn probe_docker(runner: &dyn CommandRunner) -> RuntimeAvailability {
    match runner.run("docker", &["--version"]) {
        Err(CommandError::NotFound) => return RuntimeAvailability::NotInstalled,
        Err(e) => return unavailable("docker", e, None),
        Ok(out) if !out.success => {
            return RuntimeAvailability::Unavailable {
                detail: first_line(&out.stderr),
            };
        }
        Ok(_) => {}
    }
    match runner.run("docker", &["info", "--format", "{{.ServerVersion}}"]) {
        Ok(out) if out.success => RuntimeAvailability::Available {
            detail: format!("docker {}", first_line(&out.stdout)),
        },
        Ok(out) => {
            let reason = first_line(&out.stderr);
            RuntimeAvailability::Unavailable {
                detail: if reason.is_empty() {
                    "docker 守护进程不可用".to_string()
                } else {
                    reason
                },
            }
        }
        Err(CommandError::NotFound) => RuntimeAvailability::NotInstalled,
        Err(e) => unavailable("docker", e, None),
    }
}

/// Node:`node --version`。
pub fn probe_node(runner: &dyn CommandRunner) -> RuntimeAvailability {
    match runner.run("node", &["--version"]) {
        Err(CommandError::NotFound) => RuntimeAvailability::NotInstalled,
        Err(e) => unavailable("node", e, None),
        Ok(out) if out.success => RuntimeAvailability::Available {
            detail: format!("node {}", first_line(&out.stdout)),
        },
        Ok(out) => RuntimeAvailability::Unavailable {
            detail: first_line(&out.stderr),
        },
    }
}

/// Python:优先 `uv`(有 lockfile、能管 Python 版本),没有再退到 `python3`。
pub fn probe_python(runner: &dyn CommandRunner) -> RuntimeAvailability {
    match runner.run("uv", &["--version"]) {
        Ok(out) if out.success => {
            return RuntimeAvailability::Available {
                detail: first_line(&out.stdout),
            };
        }
        Ok(_) | Err(CommandError::NotFound) => {}
        Err(e) => return unavailable("uv", e, None),
    }
    match runner.run("python3", &["--version"]) {
        Err(CommandError::NotFound) => RuntimeAvailability::NotInstalled,
        Err(e) => unavailable("python3", e, None),
        // 老版本 python 把版本号打到 stderr
        Ok(out) if out.success => {
            let line = if out.stdout.trim().is_empty() {
                first_line(&out.stderr)
            } else {
                first_line(&out.stdout)
            };
            RuntimeAvailability::Available { detail: line }
        }
        Ok(out) => RuntimeAvailability::Unavailable {
            detail: first_line(&out.stderr),
        },
    }
}

/// 一次探测全部(Settings 展示用)。`static_web` 不需要任何外部运行时,不在其中。
pub fn probe_all(runner: &dyn CommandRunner) -> Vec<(&'static str, RuntimeAvailability)> {
    vec![
        ("docker", probe_docker(runner)),
        ("node", probe_node(runner)),
        ("python", probe_python(runner)),
    ]
}
```

`$SCRATCH/a1/impl_manager.rs`:

```rust
//! `AppManager`:安装、启动、停止、卸载、启动对账——把注册表、gateway 和生命周期纯函数接起来。
//!
//! 一期只实现 `static_web`;其他 runtime 在 `install_plan` 就被明确拒绝(`UnsupportedRuntime`)。
//! 所有会改状态的方法都在同一把锁里执行(**单写者**:规格里 supervisor 是唯一写者),
//! 每次状态变化都先落盘(`AppRecord.observed`)再发事件。耗时任务句柄/进度(规格 §4.3)留到
//! 有真正耗时的 runtime 时再加:静态应用的安装是瞬时的。
//!
//! 防 TOCTOU 的安装顺序(见 `digest` 模块文档):**先把包拷进 staging,对 staging 副本重新算摘要并
//! `verify`,再改名落位**;`current_version` 在最后才写。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use crate::digest::{data_store_id_hex, digest_tree, sha256_hex};
use crate::event::AppEvent;
use crate::gateway::{Gateway, csp_for};
use crate::id::{AppId, Version};
use crate::manifest::{Manifest, ManifestError, Runtime};
use crate::plan::{
    ApprovedInstallPlan, InstallPlan, Installed, PlanInput, Provenance, TrustLevel, VerifyError,
};
use crate::registry::{AppRecord, RECORD_FORMAT_VERSION, Registry, UninstallMode, VersionRecord};
use crate::runtime::enforcement_for;
use crate::state::{
    Action, DesiredState, ObservedState, next_action, recover_after_supervisor_restart,
};

/// 应用从哪来。一期只有本地目录(目录里要有 `manifest.toml`);压缩包/仓库以后再加。
#[derive(Debug, Clone)]
pub enum AppSource {
    LocalDir(PathBuf),
}

#[derive(Debug)]
pub enum ManagerError {
    Io(io::Error),
    Manifest(ManifestError),
    Verify(VerifyError),
    /// 一期只支持 `static_web`。
    UnsupportedRuntime(String),
    /// 这个版本已经装过了(同版本不覆盖)。
    AlreadyInstalled(Version),
    /// 应用正在运行/过渡中,升级前要先停止。
    Busy(AppId),
    NotInstalled(AppId),
    /// 当前状态不允许这个操作。
    BadState {
        app: AppId,
        state: ObservedState,
    },
    /// 应用包里没有 `runtime.source` 指向的目录。
    MissingSource(PathBuf),
}

impl std::fmt::Display for ManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O 错误: {e}"),
            Self::Manifest(e) => write!(f, "{e}"),
            Self::Verify(e) => write!(f, "安装被拒绝: {e}"),
            Self::UnsupportedRuntime(k) => {
                write!(f, "暂不支持 {k} 类型的应用(一期只支持 static_web)")
            }
            Self::AlreadyInstalled(v) => write!(f, "版本 {v} 已经安装"),
            Self::Busy(id) => write!(f, "应用 {id} 正在运行,请先停止再升级"),
            Self::NotInstalled(id) => write!(f, "应用 {id} 没有安装"),
            Self::BadState { app, state } => {
                write!(f, "应用 {app} 当前状态 {state:?} 不允许这个操作")
            }
            Self::MissingSource(p) => write!(f, "应用包里缺少站点目录: {}", p.display()),
        }
    }
}

impl std::error::Error for ManagerError {}

impl From<io::Error> for ManagerError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ManifestError> for ManagerError {
    fn from(e: ManifestError) -> Self {
        Self::Manifest(e)
    }
}

/// 列表里的一项。
#[derive(Debug, Clone, PartialEq)]
pub struct AppSummary {
    pub id: AppId,
    pub name: String,
    pub version: Version,
    pub desired: DesiredState,
    pub observed: ObservedState,
    /// 运行中才有:不含令牌的站点地址。
    pub url: Option<String>,
}

pub struct AppManager {
    registry: Registry,
    gateway: Arc<Gateway>,
    host_version: Version,
    events: broadcast::Sender<AppEvent>,
    /// 单写者锁:所有改状态的方法都先拿它。
    lock: Mutex<()>,
}

/// 一个已读入并校验过的应用包。
struct Package {
    manifest: Manifest,
    manifest_text: String,
    manifest_digest: String,
    source_digest: String,
}

fn read_package(dir: &Path, host_version: &Version) -> Result<Package, ManagerError> {
    let manifest_text = fs::read_to_string(dir.join("manifest.toml"))?;
    let manifest = Manifest::from_toml(&manifest_text, host_version)?;
    Ok(Package {
        manifest_digest: sha256_hex(manifest_text.as_bytes()),
        source_digest: digest_tree(dir)?,
        manifest,
        manifest_text,
    })
}

fn static_source(manifest: &Manifest) -> Result<&str, ManagerError> {
    match &manifest.runtime {
        Runtime::StaticWeb { source } => Ok(source),
        other => Err(ManagerError::UnsupportedRuntime(
            other.kind_name().to_string(),
        )),
    }
}

/// 递归拷贝目录。只拷普通文件和目录:符号链接/FIFO/设备一律拒绝(与 `digest_tree` 同口径)。
fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &to)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("应用包里只允许普通文件和目录: {}", entry.path().display()),
            ));
        }
    }
    Ok(())
}

impl AppManager {
    pub fn new(
        root: impl Into<PathBuf>,
        host_version: Version,
        gateway: Arc<Gateway>,
    ) -> io::Result<Self> {
        let (events, _) = broadcast::channel(256);
        Ok(Self {
            registry: Registry::open(root)?,
            gateway,
            host_version,
            events,
            lock: Mutex::new(()),
        })
    }

    pub fn events(&self) -> broadcast::Receiver<AppEvent> {
        self.events.subscribe()
    }

    fn emit(&self, event: AppEvent) {
        // 没有订阅者时 send 会返回错误——不是问题
        let _ = self.events.send(event);
    }

    /// 首次导航用的地址(带一次性换 Cookie 的令牌)。**秘密,不要写日志、不要广播。**
    pub fn launch_url(&self, id: &AppId) -> String {
        self.gateway.launch_url(id)
    }

    pub fn install_plan(
        &self,
        source: &AppSource,
        provenance: Provenance,
        trust: TrustLevel,
    ) -> Result<InstallPlan, ManagerError> {
        let AppSource::LocalDir(dir) = source;
        let package = read_package(dir, &self.host_version)?;
        self.plan_for(&package, provenance, trust)
    }

    fn plan_for(
        &self,
        package: &Package,
        provenance: Provenance,
        trust: TrustLevel,
    ) -> Result<InstallPlan, ManagerError> {
        static_source(&package.manifest)?;
        let record = self.registry.load(&package.manifest.id)?;
        let installed = record.as_ref().map(|r| Installed {
            version: &r.current_version,
            grants: &r.grants,
        });
        Ok(InstallPlan::build(PlanInput {
            manifest: &package.manifest,
            manifest_digest: package.manifest_digest.clone(),
            source_digest: package.source_digest.clone(),
            provenance,
            trust,
            enforcement: enforcement_for(&package.manifest.runtime),
            installed,
        }))
    }

    /// 安装一份**已批准**的计划。**一切决定都只基于 staging 里的副本**:源目录只被"拷贝"这一个动作读取,
    /// 拷完之后它再怎么变都与本次安装无关;对 staging 副本重新计算并 `verify`,通过才落位。
    pub fn install(
        &self,
        approved: &ApprovedInstallPlan,
        source: &AppSource,
        now_ms: u64,
    ) -> Result<(), ManagerError> {
        let _guard = self.lock.lock().expect("manager 锁");
        let AppSource::LocalDir(dir) = source;
        let staging = self
            .registry
            .paths()
            .apps_dir()
            .join(format!(".staging-{}", uuid::Uuid::new_v4().simple()));
        let result = self.install_staged(approved, dir, &staging, now_ms);
        // 成功时 staging 已经改名走了,这里是空操作;失败时清掉
        if staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    fn install_staged(
        &self,
        approved: &ApprovedInstallPlan,
        dir: &Path,
        staging: &Path,
        now_ms: u64,
    ) -> Result<(), ManagerError> {
        copy_tree(dir, staging)?;
        let package = read_package(staging, &self.host_version)?;
        let id = package.manifest.id.clone();
        let version = package.manifest.version;
        let plan = self.plan_for(&package, approved.plan().provenance, approved.plan().trust)?;
        let verified = approved.verify(plan).map_err(ManagerError::Verify)?;

        let existing = self.registry.load(&id)?;
        if let Some(r) = &existing {
            if r.versions.iter().any(|v| v.version == version)
                || self.registry.paths().package_dir(&id, &version).exists()
            {
                return Err(ManagerError::AlreadyInstalled(version));
            }
            if !matches!(
                r.observed,
                ObservedState::Installed | ObservedState::Stopped | ObservedState::Failed { .. }
            ) {
                return Err(ManagerError::Busy(id));
            }
        }

        let final_dir = self.registry.paths().package_dir(&id, &version);
        fs::create_dir_all(final_dir.parent().expect("package_dir 有父目录"))?;
        fs::rename(staging, &final_dir)?;
        // 写下去的是 staging 副本里(已被 verify 的)那份 manifest,而不是源目录里此刻的内容
        self.registry.save_manifest(&id, &package.manifest_text)?;

        let upgrading = existing.is_some();
        let mut record = existing.unwrap_or_else(|| AppRecord {
            format_version: RECORD_FORMAT_VERSION,
            id: id.clone(),
            desired: DesiredState::Stopped,
            observed: ObservedState::Installed,
            current_version: version,
            grants: verified.requested,
            versions: Vec::new(),
            data_store_id: data_store_id_hex(&id),
        });
        record.grants = verified.requested;
        record.versions.push(VersionRecord {
            version,
            manifest_digest: verified.manifest_digest.clone(),
            source_digest: verified.source_digest.clone(),
            installed_ms: now_ms,
        });
        if upgrading && matches!(record.observed, ObservedState::Failed { .. }) {
            record.observed = ObservedState::Installed;
        }
        // current_version 最后才写:任何一步中途失败,旧版本仍然完整
        record.current_version = version;
        self.registry.save(&record)?;

        self.emit(AppEvent::Installed { app: id.clone() });
        if upgrading && !verified.permission_diff.is_empty() {
            self.emit(AppEvent::ManifestChanged {
                app: id,
                permission_changes: verified.permission_diff,
            });
        }
        Ok(())
    }

    fn load_record(&self, id: &AppId) -> Result<AppRecord, ManagerError> {
        self.registry
            .load(id)?
            .ok_or_else(|| ManagerError::NotInstalled(id.clone()))
    }

    fn set_observed(
        &self,
        record: &mut AppRecord,
        observed: ObservedState,
    ) -> Result<(), ManagerError> {
        record.observed = observed.clone();
        self.registry.save(record)?;
        self.emit(AppEvent::StateChanged {
            app: record.id.clone(),
            state: observed,
        });
        Ok(())
    }

    /// 启动:把应用的静态站点注册进 gateway。返回不含令牌的站点地址。
    pub fn start(&self, id: &AppId) -> Result<String, ManagerError> {
        let _guard = self.lock.lock().expect("manager 锁");
        self.start_locked(id)
    }

    fn start_locked(&self, id: &AppId) -> Result<String, ManagerError> {
        let mut record = self.load_record(id)?;
        match &record.observed {
            ObservedState::Running => return Ok(self.gateway.site_url(id)),
            ObservedState::Installed | ObservedState::Stopped => {}
            ObservedState::Failed {
                retryable: true, ..
            } => {}
            other => {
                return Err(ManagerError::BadState {
                    app: id.clone(),
                    state: other.clone(),
                });
            }
        }
        let text = fs::read_to_string(self.registry.paths().manifest_path(id))?;
        let manifest = Manifest::from_toml(&text, &self.host_version)?;
        let source = static_source(&manifest)?.to_string();
        let root = self
            .registry
            .paths()
            .package_dir(id, &record.current_version)
            .join(&source);
        if !root.is_dir() {
            let reason = format!("应用包里缺少站点目录 {source}");
            self.set_observed(
                &mut record,
                ObservedState::Failed {
                    reason,
                    retryable: false,
                },
            )?;
            return Err(ManagerError::MissingSource(root));
        }
        self.gateway.add_site(id, root, csp_for(&record.grants));
        record.desired = DesiredState::Running;
        self.set_observed(&mut record, ObservedState::Running)?;
        let url = self.gateway.site_url(id);
        self.emit(AppEvent::EndpointChanged {
            app: id.clone(),
            url: Some(url.clone()),
        });
        Ok(url)
    }

    /// 停止(也用来把 `Failed` 复位成 `Stopped`)。对已经停止的应用是空操作。
    pub fn stop(&self, id: &AppId) -> Result<(), ManagerError> {
        let _guard = self.lock.lock().expect("manager 锁");
        self.stop_locked(id)
    }

    fn stop_locked(&self, id: &AppId) -> Result<(), ManagerError> {
        let mut record = self.load_record(id)?;
        let was_serving = self.gateway.remove_site(id);
        record.desired = DesiredState::Stopped;
        match record.observed {
            ObservedState::Running | ObservedState::Failed { .. } => {
                self.set_observed(&mut record, ObservedState::Stopped)?;
            }
            _ => self.registry.save(&record)?,
        }
        if was_serving {
            self.emit(AppEvent::EndpointChanged {
                app: id.clone(),
                url: None,
            });
        }
        Ok(())
    }

    /// 卸载(不存在不算错误)。运行中先停。
    pub fn uninstall(&self, id: &AppId, mode: UninstallMode) -> Result<(), ManagerError> {
        let _guard = self.lock.lock().expect("manager 锁");
        if self.registry.load(id)?.is_none() {
            self.registry.uninstall(id, mode)?;
            return Ok(());
        }
        self.stop_locked(id)?;
        self.registry.uninstall(id, mode)?;
        self.emit(AppEvent::StateChanged {
            app: id.clone(),
            state: ObservedState::NotInstalled,
        });
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<AppSummary>, ManagerError> {
        let listing = self.registry.list()?;
        Ok(listing
            .apps
            .into_iter()
            .map(|r| {
                let name = fs::read_to_string(self.registry.paths().manifest_path(&r.id))
                    .ok()
                    .and_then(|t| Manifest::from_toml(&t, &self.host_version).ok())
                    .map(|m| m.name)
                    .unwrap_or_else(|| r.id.to_string());
                let url = matches!(r.observed, ObservedState::Running)
                    .then(|| self.gateway.site_url(&r.id));
                AppSummary {
                    name,
                    version: r.current_version,
                    desired: r.desired,
                    observed: r.observed,
                    url,
                    id: r.id,
                }
            })
            .collect())
    }

    /// supervisor 启动时调用:把持久化的观察态修正为现实(应用随 supervisor 一起停了),再按 `desired` 对账。
    /// 返回失败的应用与原因,一个应用失败不影响其他应用。
    pub fn reconcile(&self) -> Vec<(AppId, ManagerError)> {
        let _guard = self.lock.lock().expect("manager 锁");
        let mut failures = Vec::new();
        let listing = match self.registry.list() {
            Ok(l) => l,
            Err(e) => return vec![(AppId::new("unknown").expect("合法"), ManagerError::Io(e))],
        };
        for mut record in listing.apps {
            let id = record.id.clone();
            let recovered = recover_after_supervisor_restart(record.observed.clone());
            if recovered != record.observed
                && let Err(e) = self.set_observed(&mut record, recovered)
            {
                failures.push((id, e));
                continue;
            }
            let result = match next_action(record.desired, &record.observed) {
                Some(Action::Start) => self.start_locked(&id).map(|_| ()),
                Some(Action::Stop) => self.stop_locked(&id),
                // 卸载只删程序(数据由用户显式要求才删)
                Some(Action::Uninstall) => self
                    .registry
                    .uninstall(&id, UninstallMode::Program)
                    .map_err(Into::into),
                None => Ok(()),
            };
            if let Err(e) = result {
                failures.push((id, e));
            }
        }
        failures
    }
}
```

- [ ] **Step 5: 装配并跑测试(默认并行度,多跑几遍)**

装配脚本 `$SCRATCH/a1_assemble.py`(带引号 heredoc):

```python
#!/usr/bin/env python3
"""A1 一次性脚本(步骤 5):把各模块的实现写到测试前面。`SP` 同上。**不提交。**"""
import os

SP = os.environ["SP"]
CRATE = "crates/bytehost-apps"
FILES = {
    "gateway_mod": "gateway/mod.rs",
    "gateway_static_files": "gateway/static_files.rs",
    "port": "port.rs",
    "runtime_mod": "runtime/mod.rs",
    "runtime_probe": "runtime/probe.rs",
    "manager": "manager.rs",
}
for name, rel in FILES.items():
    impl = open(os.path.join(SP, f"impl_{name}.rs")).read()
    tests = open(os.path.join(SP, f"tests_{name}.rs")).read()
    open(f"{CRATE}/src/{rel}", "w").write(impl + tests)
print("ok")
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a1 && export PYTHONDONTWRITEBYTECODE=1
SP=$SCRATCH/a1 python3 $SCRATCH/a1_assemble.py && cargo fmt -p bytehost-apps && git status --short
cargo test -p bytehost-apps --features server 2>&1 | grep -E "^error|FAILED|test result"
for i in 1 2 3 4 5; do cargo test -p bytehost-apps --features server 2>&1 | grep -E "FAILED|test result"; done
```
Expected: 装配脚本输出 `ok`;`git status --short` 是 `M Cargo.lock`、`M crates/bytehost-apps/Cargo.toml`、`M crates/bytehost-apps/src/lib.rs`、以及 `??` 的 `src/gateway/`、`src/manager.rs`、`src/port.rs`、`src/runtime/`、`src/testutil.rs`;第一次运行 `test result: ok. 118 passed; 0 failed`,**后面 5 次每次都是 118 passed、没有挂起**(若某次卡住,先用 `perl -e 'alarm 60; exec @ARGV' <测试二进制>` 加超时,再定位挂起的测试——不要用 `--test-threads=1` 绕过去)。

- [ ] **Step 6: 每个 feature 组合、clippy、依赖门禁、锁文件**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a1
for f in "" "--features digest" "--features manifest-toml" "--features server" "--all-features"; do printf "[%s] " "$f"; cargo test -p bytehost-apps $=f 2>&1 | grep -E "^error|test result: .* [1-9][0-9]* passed" | head -2; done
cargo clippy -p bytehost-apps --all-targets --all-features 2>&1 | grep -E "^(warning|error)" | head -3; echo "clippy(all) done"
cargo clippy -p bytehost-apps --all-targets 2>&1 | grep -E "^(warning|error)" | head -3; echo "clippy(default) done"
./scripts/check-bytehost-apps-deps.sh
git diff Cargo.lock | grep '^[-+]' | grep -v '^+++\|^---'
```
Expected: 默认 **55**、`digest` **63**、`manifest-toml` **61**、`server` **118**、`--all-features` **118**;两次 clippy 都**没有**任何 `warning`/`error` 行;门禁 `bytehost-apps deps check: ok`(它用 `--all-features`,允许 tokio/hyper,只禁 `dozer*`;默认 feature 闭包仍是 serde 家族);`git diff Cargo.lock` 里**没有任何 `-` 行**,`+` 行是 `bytehost-apps` 块里的 `bytes`/`http-body-util`/`hyper`/`hyper-util`/`tokio`/`uuid` 与一个新的 `httpdate` `[[package]]`。

- [ ] **Step 7: 变异检验(先暂存)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a1 && export PYTHONDONTWRITEBYTECODE=1
git add -A crates Cargo.lock
cat > $SCRATCH/a1_mutate.py <<'EOF'
import sys
w = sys.argv[1]
S = "crates/bytehost-apps/src/"
M = {
 "1": (S+"gateway/mod.rs", "if host_port.parse::<u16>().ok()? != port {", "if false {"),
 "2": (S+"gateway/mod.rs", "    if !authed {", "    if false {"),
 "3": (S+"gateway/mod.rs", ".is_some_and(|v| ct_eq(v, &state.token));", ".is_some();"),
 "4": (S+"gateway/static_files.rs", '".." => return Resolved::Forbidden,', '".." => {}'),
 "5": (S+"gateway/static_files.rs", "if !real.starts_with(&real_root) {", "if false {"),
 "6": (S+"gateway/mod.rs", "Outbound::Any => None,", 'Outbound::Any => Some("x".to_string()),'),
 "7": (S+"gateway/mod.rs", "if method != Method::GET && method != Method::HEAD {", "if false {"),
 "8": (S+"manager.rs", "let verified = approved.verify(plan).map_err(ManagerError::Verify)?;", "let verified = plan;"),
 "9": (S+"manager.rs", "        if staging.exists() {\n            let _ = fs::remove_dir_all(&staging);\n        }", "        let _ = &staging;"),
 "10": (S+"manager.rs", "let recovered = recover_after_supervisor_restart(record.observed.clone());", "let recovered = record.observed.clone();"),
 "11": (S+"port.rs", "PORT_MIN + n %", "1 + n %"),
 "12": (S+"gateway/mod.rs", "self.shutdown.notify_one();", "self.shutdown.notify_waiters();"),
 "13": (S+"manager.rs", "let was_serving = self.gateway.remove_site(id);", "let was_serving = false;"),
 "14": (S+"gateway/static_files.rs", "let Some(decoded) = percent_decode(url_path) else {\n        return Resolved::BadRequest;\n    };", "let decoded = url_path.to_string();"),
 "15": (S+"runtime/probe.rs", 'Ok(out) => {\n            let reason = first_line(&out.stderr);', 'Ok(_out) if true => RuntimeAvailability::Available { detail: "docker x".into() },\n        Ok(out) => {\n            let reason = first_line(&out.stderr);'),
 "16": (S+"manager.rs", "            if r.versions.iter().any(|v| v.version == version)\n                || self.registry.paths().package_dir(&id, &version).exists()\n            {", "            if false {"),
}
p, a, b = M[w]
s = open(p).read()
assert a in s, (w, a[:60])
open(p, "w").write(s.replace(a, b, 1))
EOF
for m in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16; do printf "M$m: "; python3 $SCRATCH/a1_mutate.py $m 2>&1 | tail -1; BIN=$(cargo test -p bytehost-apps --features server --no-run 2>&1 | grep -o 'target/[^)]*bytehost_apps-[a-f0-9]*' | head -1); r=$(perl -e 'alarm 60; exec @ARGV' $BIN --test-threads=4 2>&1 | grep -E "FAILED$|^error" | head -2 | tr '\n' '|'); echo "${r:-NOT CAUGHT}"; git checkout -- crates; done
git diff --stat | wc -l
```
Expected: 十六次变异**每一次**都以 `FAILED` 收场(没有 `NOT CAUGHT`;`M12` 的失败是 `shutdown` 超时 panic,需要几秒):M1(Host 不查端口):`the_host_header_must_be_exactly…`;M2(令牌检查去掉):`without_the_token_every_request_is_403…`;M3(Cookie 只要存在不比值):同上;M4(`..` 段放行):`no_spelling_of_a_parent_reference…`;M5(不校验规范化路径在根内):`a_symlink_pointing_outside_the_root_is_forbidden`;M6(`any` 也给 CSP):`the_csp_follows_the_granted_outbound_network_permission`;M7(不限方法):`only_get_and_head_are_allowed…`;M8(`verify` 跳过):`a_forged_approved_payload…`;M9(staging 不清理):`every_failed_install_cleans_up_and_the_saved_manifest_is_the_staged_one`;M10(对账不先修正观察态):`after_a_supervisor_restart_reconcile…`;M11(端口落到范围外):`picked_ports_stay_inside_the_dynamic_range`;M12(`notify_waiters`):`shutdown_never_hangs…`;M13(停止不注销站点):`install_start_serve_and_stop_round_trip…`;M14(不做百分号解码):`no_spelling…`;M15(守护进程报错仍算可用):`docker_is_reported_in_three_distinct_layers`;M16(同版本可覆盖):`the_same_version_is_never_overwritten…`;每次还原后 `git diff --stat | wc -l` 为 `0`。

- [ ] **Step 8: 全 workspace 没被波及,然后提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a1
cargo check -p dozer-core -p dozerd 2>&1 | grep -E "^error|Finished" | head -3
git status --short
git add crates/bytehost-apps Cargo.lock
git diff --cached --stat | tail -14
git commit -m "feat(bytehost-apps): server feature — gateway, AppManager (static_web), port persistence, runtime probes

A1 of the bytehost app host. One fixed-port gateway serves every app by Host header
(<app-id>.localhost:<port>) with Host validation, a per-process session token exchanged for a
host-only HttpOnly/SameSite=Strict cookie, path resolution that cannot leave the site root, and a CSP
derived from the granted outbound-network permission. AppManager installs from a staged copy (digest
re-verified on the copy), starts/stops static sites, uninstalls (program vs data) and reconciles after a
supervisor restart. The port is chosen randomly once and persisted; a taken port is an error, never a
silent change. docker/node/python are probed in layers (not installed / unavailable / available).

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: `cargo check` 以 `Finished` 结束;暂存区只有 `crates/bytehost-apps/**`(`Cargo.toml`、`src/lib.rs`、`src/testutil.rs`、`src/gateway/{mod,static_files}.rs`、`src/port.rs`、`src/runtime/{mod,probe}.rs`、`src/manager.rs`)与 `Cargo.lock`;**没有** `docs/`、`CLAUDE.md`、`.cargo/`。

---

### Task 2: 文档回填

**Files:**
- Modify: `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`、`docs/superpowers/specs/2026-10-04-bytehost-extraction-roadmap.md`、`CLAUDE.md`

**Interfaces:**
- Consumes: Task 1 的提交。

- [ ] **Step 1: 回填并校验**

用带引号的 heredoc 创建 `$SCRATCH/a1_docs.py`(提交短 id 经环境变量传入):

```python
import os
A1 = os.environ["A1"]
sp = "docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md"
lines = open(sp).read().split("\n")
hit = False
for k, l in enumerate(lines):
    if l.startswith("| **A1** |"):
        lines[k] = l[:-1].rstrip() + " **已完成(A1,`bytehost-a1`):`" + A1 + "`**——`server` feature:`AppManager`、`static_web`、gateway(Host 校验 + 会话令牌 + 路径解析 + CSP)、端口持久化、docker/node/python 分层探测;V1 已实测通过 |"
        hit = True
assert hit
open(sp, "w").write("\n".join(lines))
rp = "docs/superpowers/specs/2026-10-04-bytehost-extraction-roadmap.md"
lines = open(rp).read().split("\n")
hit = False
for k, l in enumerate(lines):
    if l.startswith("- **A1** "):
        lines[k] = l + " **已完成(`bytehost-a1`):`" + A1 + "`。**"
        hit = True
assert hit
open(rp, "w").write("\n".join(lines))
cp = "CLAUDE.md"
s = open(cp).read()
a = "门禁 `scripts/check-bytehost-apps-deps.sh` |\n"
assert a in s
s = s.replace(a, "门禁 `scripts/check-bytehost-apps-deps.sh`。`server` feature(gateway/`AppManager`/runtime 探测;tokio + hyper)只有 dozerd 打开 |\n", 1)
open(cp, "w").write(s)
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a1 && export PYTHONDONTWRITEBYTECODE=1
A1=$(git log --format=%h -1 --grep="server feature") python3 $SCRATCH/a1_docs.py
git diff -- docs CLAUDE.md | grep -E "^[-+]" | grep -vE "^(\+\+\+|---)" | cut -c1-170
./scripts/check-bytehost-apps-deps.sh
```
Expected: `git diff` 里只有:应用宿主规格里 A1 行的"已完成"标记、路线图 A1 行的"已完成"标记、`CLAUDE.md` 里 `crates/bytehost-apps` 一行末尾多出关于 `server` feature 的说明;**反引号内容完整**;门禁 `ok`。

- [ ] **Step 2: 提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a1
git status --short
git add docs CLAUDE.md
git diff --cached --stat | tail -5
git commit -m "docs(bytehost): backfill A1 (server feature: gateway, AppManager, runtime probes)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: 只有 `docs/` 下 2 个文件与 `CLAUDE.md`。

---

## Self-Review

**1. 覆盖:** 规格 §7 的 A1 项——`AppManager` ✔、`static_web` runtime ✔、gateway(Host 校验、静态文件、固定端口)✔(外加会话令牌与 CSP,规格 §5.2 第 4 点)、各 runtime 的 `probe`(docker/colima、node、uv 分层)✔、端口策略(A12)✔、强制等级表(规格 §4.2)✔、V1 ✔(已先实测)。**未覆盖并在 Review Focus 9 说明:** 压缩包来源、进程型 runtime 的运行、WebSocket 代理、dozerd 接线(A2)、GUI(A4)。

**2. 占位符扫描:** 测试、实现、脚本都是草稿里跑通的完整版本(草稿在全新 worktree 里用同一批文件重放:默认/`digest`/`manifest-toml`/`server`/全部 feature 的测试数 55/63/61/118/118,clippy 零诊断,门禁 ok,`Cargo.lock` 仅 13 行新增、无删除;16 个变异均被对应测试抓到)。

**3. 一致性:** `Gateway`/`GatewayConfig`/`csp_for`/`AppManager`/`AppSource`/`AppSummary`/`ManagerError`/`pick_port`/`load_or_choose_port`/`enforcement_for`/`RuntimeAvailability`/`CommandRunner` 的名字与形状在测试、实现、Interfaces、文档里一致。`install` 接收 `now_ms` 而不是自己读时钟——单测可控,时间由调用方(dozerd)提供。

**4. Review Focus:** 9 条各有归属(1→测试+变异 12;2→事件无令牌的断言;3、4→说明与 A5;5→测试+变异 9;6、7→A2 注意事项;8→探测测试;9→范围说明)。
