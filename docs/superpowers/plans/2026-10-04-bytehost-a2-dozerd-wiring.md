# bytehost 应用宿主 A2:接入 dozerd——线上协议、`AppService`、启动对账与退出收尾、`dozer-client` 方法 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地应用宿主规格(`docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`)§6.1、§7 的 **A2** 切片,把 A0/A1 做好的 `AppManager` 与 gateway 接进 dozerd——**应用跟随 dozerd**(用户 2026-10-04 已定):
1. 线上类型 `AppRequest`/`AppReply` 放在 `bytehost-apps::proto`(默认 feature,只依赖 serde),`dozer-core::protocol` 加 `Request::App`/`Reply::App` 原样带过去;
2. dozerd 里的 `AppService`:启动**永不失败**(端口被占用等只让它"不可用"并说明原因)、启动时按 `desired` 对账、`Shutdown`/Ctrl-C 时撤下站点并**保留 `desired`**、每次调用经 `spawn_blocking`;
3. `dozer-client` 的 `app_*` 方法;
4. 依赖门禁新增"`dozer-hook` 的依赖闭包不得被 bytehost-apps 的可选依赖污染"。
**GUI 不在本切片**(A3/A4);也**没有推送事件**——GUI 在 A4 之前靠轮询 `List`。

**Architecture:** 一个代码任务 + 一个文档任务。Task 1 测试先行:`a2_tests.py` 写入全部测试(两个新文件先只含测试、三处已有文件追加测试、声明两个新模块),编译失败(RED);再跑 `a2_impl.py`(新文件的实现 + 对已有文件的 assert 式替换 + 所有构造 `Stores` 的测试字面量补 `apps`);然后全 workspace 检查、clippy、门禁,8 个变异检验 + 1 个门禁变异。Task 2 回填文档。

**Tech Stack:** Rust(`bytehost-apps`、`dozer-core`、`dozerd`、`dozer-client`);Python 3 标准库(一次性脚本)。`dozer-core`/`dozer-client` 新增依赖 `bytehost-apps`(**默认 feature,不含 tokio/hyper/sha2/toml**);`dozerd` 新增 `bytehost-apps` 的 `server` feature。`Cargo.lock` 只新增这几条依赖边,没有新的第三方包。

**Spec:** 应用宿主规格 §3.2(依赖方向与线上协议)、§6.1(应用跟随 dozerd;gateway 放进 dozerd)、§7(A2);路线图阶段 1。

## Global Constraints

- **`dozer-hook`/`dozer-mcp` 的依赖闭包不得多出任何第三方 crate**(只多一个 `bytehost-apps` 自己)。由 `scripts/check-bytehost-apps-deps.sh`(本计划扩展了第 3 项检查)守住。
- **`bytehost-apps` 里不得出现 `dozer`/`Dozer` 字样的类型或路径**;线上类型不知道 socket 长什么样。
- **在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytehost-a2/...`),分支 `bytehost-a2`,从当前 `main` 开。主 checkout 常有并发会话与未提交改动(此刻有别的会话的 PlantUML 工作),不得在其上改代码或 `git add -A`;**更不要在主 checkout 里 `git checkout -- Cargo.lock`**(那会丢掉别的会话尚未提交的锁文件改动)。只 `git add` 指定路径;提交前 `git status` 与 `git diff --cached --stat` 看全貌。
- **worktree 里不要复制 `.cargo/config.toml`**(同 A0/A1):`Cargo.lock` 保持干净,本切片的锁文件改动(只有依赖边)直接提交;提交前 `git diff Cargo.lock` 确认**没有 `-` 行**。
- **变异检验前先 `git add` 暂存当前版本**(新文件也要 `git add`),变异后用 `git checkout -- crates scripts` 还原。
- **每次改 Rust 后跑 `cargo fmt --all`;变异脚本匹配的是 `cargo fmt` 之后的源码。**
- **`cargo` 报 `sccache: Operation not permitted` 时**(只在主 checkout 出现):加 `RUSTC_WRAPPER=` 前缀;worktree 里不会遇到。
- **涉及"端口是否关闭"的断言一律走 `Gateway::is_stopped()`/`AppService::gateway_stopped()`,不要再连一次端口**——端口一释放就可能被别的程序(或并行的别的测试)立刻重新占用,连接探测会误判(草稿里第一版测试就因此偶发失败)。
- **heredoc 一律用带引号的分隔符(`<<'EOF'`);或直接用编辑器/Write 工具写文件。**
- 提交信息用 `feat:`/`test:`/`docs:` 前缀;末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知基线

- `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 稳定失败(`dozer-app`);`assets::tests::serves_vendored_asset_with_mime` 偶发失败。
- 草稿结果(`main` = A1 之后):`bytehost-apps` 默认 feature **55 → 59**、`--all-features` **126 → 132**;`dozer-core` **97 → 98**;`dozerd --lib` **463 → 471**;新增集成测试 `dozerd --test app_requests` **3**;`dozer-client`(4/14/1/1)、`dozer-mcp`、`dozer-hook`、`dozer-app`(1776 + 已知失败)**不变**。`cargo check --workspace --all-targets` 通过。clippy:这四个 crate 在 `--all-targets --all-features` 下只剩 `dozerd/src/preview_commands.rs` 里一条**既有**诊断(与本切片无关)。

## Review Focus

1. **退出收尾只覆盖两条路径:** `Request::Shutdown`(Settings 里"停止 dozerd")和 Ctrl-C(SIGINT)。dozerd 被 SIGTERM/`kill -9`/崩溃杀掉时**不会**收尾——`state.json` 里观察态仍是 `Running`,下次启动靠 `reconcile` 里的 `recover_after_supervisor_restart` 修正(A0/A1 已有测试固定)。所以崩溃安全靠对账,不靠收尾。
2. **"不可用"是终态,直到 dozerd 重启:** gateway 端口被占用、端口文件坏了、应用目录打不开,都会让 `AppService` 永久不可用(每个请求回带原因的错误),**不会自动重试**。这是有意的:自动换端口会让所有应用丢本地数据。修改端口的入口属于 A4 的 Settings。**首次运行时端口恰好被占用**(持久化发生在首次绑定之前)的恢复也留给 A4 前的小修(A1 评审已记)。
3. **阻塞 I/O:** `AppManager` 的每个方法都经 `spawn_blocking`(单写者 `std::sync::Mutex` 不会阻塞 tokio 工作线程);但 `AppService::start_with` 里的 `AppManager::new`(建应用目录)在 async 上下文里同步执行——只有几个 `create_dir_all`,启动时一次,可以接受。
4. **令牌不得泄露:** `launch_url` 是秘密,只通过 `Reply::App { LaunchUrl }` 回给请求方;事件日志(`AppService` 起的后台任务)只记 `AppEvent`,其中的站点地址**不含令牌**。审阅时确认 `server.rs` 里没有任何地方把整条 `Reply` 以 debug 级别打进日志(草稿里没有)。
5. **协议是增量的:** `Request::App`/`Reply::App` 是新增变体。新 GUI 连旧 dozerd 会得到"协议错误"的通用错误;旧 GUI 连新 dozerd 不受影响。没有协议版本号机制(沿用现状)。
6. **所有构造 `dozerd::server::Stores` 的地方都要补 `apps`:** 24 处测试字面量(dozerd、dozer-client、dozer-mcp 的 `tests/`)由脚本补成 `AppService::unavailable("test")`;今后新增的构造点同理。
7. **没有推送事件:** `AppManager` 有 `events()` 广播,dozerd 里只起了一个把事件写进日志的后台任务;GUI 订阅状态变化(`Attach` 式的流)留给 A4——在此之前 GUI 轮询 `List`。
8. **数据位置:** `AppService::start` 的根目录是 `dozer_core::paths::state_dir()/bytehost`(与 `dozer.db` 同目录;macOS 上 `state_dir()` 回退到 `data_local_dir()`)。卸载"仅程序"保留的 `data/` 就在这里。
9. **没做的(范围外):** GUI(A3/A4)、推送事件、运行时安装(二期)、`proto` 里的版本协商、Windows/Linux 下 `state_dir` 的差异。

---

## File Structure

| 路径 | 职责 | 任务 |
|---|---|---|
| `crates/bytehost-apps/src/proto.rs`(新) | `AppRequest`/`AppReply`、`AppSource`/`AppSummary`/`RuntimeAvailability`/`RuntimeProbe`(从 `manager`/`probe` 搬来) + 4 个测试 | 1 |
| `crates/bytehost-apps/src/{lib,registry,manager,gateway/mod,runtime/probe}.rs` | `HOST_VERSION`;`UninstallMode` 可序列化;类型搬家与 `LocalDir` 变成结构体变体;`AppManager::suspend_all`;`Gateway::{stop,is_stopped}` + 各 1 个测试 | 1 |
| `crates/dozer-core/{Cargo.toml,src/protocol.rs}` | 依赖 `bytehost-apps`;`Request::App`/`Reply::App` + 1 个测试 | 1 |
| `crates/dozerd/src/app_service.rs`(新) | `AppService`:启动不失败、对账、`handle`、`shutdown`、`gateway_stopped` + 8 个测试 | 1 |
| `crates/dozerd/src/{lib,server,main}.rs`、`Cargo.toml` | `Stores.apps`;`Request::App` 分派;`Shutdown`/Ctrl-C 收尾;启动 | 1 |
| `crates/dozerd/tests/app_requests.rs`(新) | 经真实 socket 与 `dozer-client` 的 3 个集成测试 | 1 |
| `crates/dozer-client/{Cargo.toml,src/lib.rs}` | `app_request` 与 8 个类型化的 `app_*` 方法 | 1 |
| 各 crate 的 `tests/*.rs`(24 处) | `Stores` 字面量补 `apps` | 1 |
| `scripts/check-bytehost-apps-deps.sh` | 新增第 3 项:`dozer-hook` 依赖闭包检查 | 1 |
| `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`、`…-extraction-roadmap.md`、`CLAUDE.md` | 回填 | 2 |

一次性脚本与测试/实现源文件(**不提交**)放 `$SCRATCH`(仓库外)。

---

### Task 1: 接入 dozerd

**Files:** 见上表。

**Interfaces:**
- Produces(A3/A4 依赖):
  - `bytehost_apps::proto::{AppRequest::{List, Plan { source, provenance, trust }, Install { approved: Box<ApprovedInstallPlan>, source }, Start { id }, Stop { id }, Uninstall { id, mode }, LaunchUrl { id }, ProbeRuntimes}, AppReply::{Apps { apps }, Plan { plan: Box<InstallPlan> }, Done, Started { url }, LaunchUrl { url }, Runtimes { runtimes }}, AppSource::LocalDir { path }, AppSummary, RuntimeAvailability, RuntimeProbe { runtime, availability /*flatten*/ }}`;`bytehost_apps::HOST_VERSION: Version`;`AppManager::suspend_all() -> Vec<(AppId, ManagerError)>`;`Gateway::{stop(&self) async, is_stopped(&self) -> bool}`。
  - `dozer_core::protocol::{Request::App { request }, Reply::App { reply }}`。
  - `dozerd::app_service::AppService::{unavailable(impl Into<String>) -> Arc<Self>, start(&Path) async -> Arc<Self>, start_with(&Path, GatewayConfig) async -> Arc<Self>, handle(AppRequest) async -> Result<AppReply, String>, shutdown(&self) async, gateway_stopped(&self) -> bool}`;`dozerd::server::Stores.apps`。
  - `dozer_client::Client::{app_request, app_list, app_plan, app_install, app_start, app_stop, app_uninstall, app_launch_url, app_probe_runtimes}`。

- [ ] **Step 1: 建 worktree、分支、脚本目录**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytehost-a2 -b bytehost-a2 main
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a2 && git log --oneline | head -1 && git status --short && ls .cargo 2>/dev/null
export SCRATCH=~/Projects/CoralProjects/byteboy/dozer-bytehost-a2-scratch && mkdir -p $SCRATCH/a2
```

Expected: 干净、分支 `bytehost-a2`、`ls .cargo` 没有输出(**不要**复制 `config.toml`)。**此后所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytehost-a2/` 前缀,且每个新 shell 先 `export SCRATCH=…`。**

- [ ] **Step 2: 记录基线**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a2
for spec in "-p bytehost-apps" "-p bytehost-apps --all-features" "-p dozer-core" "-p dozerd --lib" "-p dozer-client"; do printf "[%s] " "$spec"; cargo test $=spec 2>&1 | grep -E "^error|FAILED|test result: .* [1-9][0-9]* passed" | awk '{print $4,$5}' | tr '\n' ' '; echo; done
cargo test -p dozer-app 2>&1 | grep -E "^error|test result|FAILED" | tee $SCRATCH/dozer-app-baseline.txt
./scripts/check-bytehost-apps-deps.sh
```
Expected: `bytehost-apps` 55 与 126;`dozer-core` 97;`dozerd --lib` 463;`dozer-client` 4/14/1/1;`dozer-app` `1776 passed; 1 failed`(失败只有 `delete_confirm…`);门禁 `ok`。**若数字不同,说明 `main` 在 A1 之后有改动,以实测为准并在后面的期望里相应调整。**

- [ ] **Step 3: 写测试(先看编译失败)**

把下面各段写成 `$SCRATCH/a2/` 下的文件,再写测试脚本 `$SCRATCH/a2_tests.py`(带引号 heredoc):

`$SCRATCH/a2/tests_proto.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id() -> AppId {
        AppId::new("excalidraw").unwrap()
    }

    fn round_trip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(
        value: &T,
    ) -> serde_json::Value {
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(&serde_json::from_value::<T>(json.clone()).unwrap(), value);
        json
    }

    #[test]
    fn simple_requests_use_a_stable_op_tag() {
        assert_eq!(round_trip(&AppRequest::List), json!({"op": "list"}));
        assert_eq!(
            round_trip(&AppRequest::ProbeRuntimes),
            json!({"op": "probe_runtimes"})
        );
        assert_eq!(
            round_trip(&AppRequest::Start { id: id() }),
            json!({"op": "start", "id": "excalidraw"})
        );
        assert_eq!(
            round_trip(&AppRequest::Stop { id: id() }),
            json!({"op": "stop", "id": "excalidraw"})
        );
        assert_eq!(
            round_trip(&AppRequest::LaunchUrl { id: id() }),
            json!({"op": "launch_url", "id": "excalidraw"})
        );
        assert_eq!(
            round_trip(&AppRequest::Uninstall {
                id: id(),
                mode: UninstallMode::ProgramAndData
            }),
            json!({"op": "uninstall", "id": "excalidraw", "mode": "program_and_data"})
        );
    }

    #[test]
    fn plan_requests_carry_the_source_provenance_and_trust() {
        let req = AppRequest::Plan {
            source: AppSource::LocalDir {
                path: "/tmp/app".into(),
            },
            provenance: Provenance::AgentGenerated,
            trust: TrustLevel::Untrusted,
        };
        assert_eq!(
            round_trip(&req),
            json!({"op": "plan", "source": {"kind": "local_dir", "path": "/tmp/app"}, "provenance": "agent_generated", "trust": "untrusted"})
        );
    }

    #[test]
    fn unknown_ops_and_unknown_sources_are_rejected_not_ignored() {
        assert!(serde_json::from_value::<AppRequest>(json!({"op": "format_disk"})).is_err());
        assert!(serde_json::from_value::<AppRequest>(json!({"op": "plan", "source": {"kind": "url", "url": "x"}, "provenance": "local", "trust": "trusted"})).is_err());
        assert!(
            serde_json::from_value::<AppRequest>(json!({"op": "start", "id": "../etc"})).is_err(),
            "非法 app id 在反序列化时就被拒绝"
        );
    }

    #[test]
    fn replies_round_trip_including_runtime_probes_with_flattened_availability() {
        round_trip(&AppReply::Done);
        round_trip(&AppReply::Started {
            url: "http://excalidraw.localhost:20001/".into(),
        });
        round_trip(&AppReply::LaunchUrl {
            url: "http://excalidraw.localhost:20001/?bh_token=t".into(),
        });
        let apps = AppReply::Apps {
            apps: vec![AppSummary {
                id: id(),
                name: "Excalidraw".into(),
                version: Version::new(0, 17, 0),
                desired: DesiredState::Running,
                observed: ObservedState::Running,
                url: Some("http://excalidraw.localhost:20001/".into()),
            }],
        };
        let json = round_trip(&apps);
        assert_eq!(json["apps"][0]["version"], "0.17.0");
        assert_eq!(json["apps"][0]["observed"], json!({"state": "running"}));

        let runtimes = AppReply::Runtimes {
            runtimes: vec![
                RuntimeProbe {
                    runtime: "docker".into(),
                    availability: RuntimeAvailability::Unavailable {
                        detail: "Colima 没启动".into(),
                    },
                },
                RuntimeProbe {
                    runtime: "node".into(),
                    availability: RuntimeAvailability::NotInstalled,
                },
            ],
        };
        let json = round_trip(&runtimes);
        assert_eq!(
            json["runtimes"][0],
            json!({"runtime": "docker", "availability": "unavailable", "detail": "Colima 没启动"})
        );
        assert_eq!(
            json["runtimes"][1],
            json!({"runtime": "node", "availability": "not_installed"})
        );
    }
}
```

`$SCRATCH/a2/tests_app_service.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;
    use bytehost_apps::id::AppId;
    use bytehost_apps::plan::{Approval, Provenance, TrustLevel};
    use bytehost_apps::proto::{AppRequest, AppSource};
    use bytehost_apps::registry::UninstallMode;
    use std::io::{Read, Write};

    fn write_app(dir: &Path, id: &str, body: &str) -> AppSource {
        std::fs::create_dir_all(dir.join("web")).unwrap();
        std::fs::write(
            dir.join("manifest.toml"),
            format!(
                r#"schema_version = 1
min_host_version = "0.1.0"
id = "{id}"
name = "{id} app"
version = "1.0.0"

[presentation]
entrypoint = "main"

[entrypoints.main]
type = "web"
path = "/"

[runtime]
kind = "static_web"
source = "web/"
"#
            ),
        )
        .unwrap();
        std::fs::write(dir.join("web/index.html"), body).unwrap();
        AppSource::LocalDir {
            path: dir.to_path_buf(),
        }
    }

    async fn install(svc: &AppService, source: AppSource) {
        let reply = svc
            .handle(AppRequest::Plan {
                source: source.clone(),
                provenance: Provenance::Local,
                trust: TrustLevel::Trusted,
            })
            .await
            .unwrap();
        let AppReply::Plan { plan } = reply else {
            panic!("{reply:?}")
        };
        let approved = plan.approve(Approval {
            approver: "test".into(),
            approved_ms: 1,
        });
        let done = svc
            .handle(AppRequest::Install {
                approved: Box::new(approved),
                source,
            })
            .await
            .unwrap();
        assert_eq!(done, AppReply::Done);
    }

    fn id(s: &str) -> AppId {
        AppId::new(s).unwrap()
    }

    /// 带着从 `launch_url` 里取出的令牌访问应用站点,返回(状态码, 正文)。
    fn fetch(launch_url: &str, id: &str) -> (u16, String) {
        let rest = launch_url.strip_prefix("http://").unwrap();
        let (authority, path_and_query) = rest.split_once('/').unwrap();
        let port: u16 = authority.rsplit_once(':').unwrap().1.parse().unwrap();
        let token = path_and_query.split("bh_token=").nth(1).unwrap();
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        write!(s, "GET / HTTP/1.1\r\nHost: {id}.localhost:{port}\r\nCookie: bh_session={token}\r\nConnection: close\r\n\r\n").unwrap();
        let mut raw = String::new();
        s.read_to_string(&mut raw).unwrap();
        let status = raw.split_whitespace().nth(1).unwrap().parse().unwrap();
        (
            status,
            raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string(),
        )
    }

    fn running_url(reply: AppReply) -> String {
        match reply {
            AppReply::LaunchUrl { url } => url,
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unavailable_service_answers_every_request_with_its_reason() {
        let svc = AppService::unavailable("原因 X");
        for req in [
            AppRequest::List,
            AppRequest::ProbeRuntimes,
            AppRequest::Stop { id: id("a") },
        ] {
            assert_eq!(svc.handle(req).await.unwrap_err(), "原因 X");
        }
        svc.shutdown().await; // 不可用的服务关停也是空操作
    }

    /// 端口被占用不能拖垮 dozerd:服务变成"不可用",并把原因(含端口)告诉请求方。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_taken_gateway_port_makes_the_service_unavailable_instead_of_failing() {
        let tmp = tempfile::tempdir().unwrap();
        let squatter = Gateway::start(GatewayConfig { port: 0 }).await.unwrap();
        let taken = squatter.port();
        let svc = AppService::start_with(tmp.path(), GatewayConfig { port: taken }).await;
        let err = svc.handle(AppRequest::List).await.unwrap_err();
        assert!(
            err.contains(&taken.to_string()) && err.contains("占用"),
            "{err}"
        );
        squatter.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unusable_application_directory_makes_the_service_unavailable() {
        let tmp = tempfile::tempdir().unwrap();
        let blocker = tmp.path().join("not-a-dir");
        std::fs::write(&blocker, "x").unwrap();
        let svc =
            AppService::start_with(&blocker.join("bytehost"), GatewayConfig { port: 0 }).await;
        let err = svc.handle(AppRequest::List).await.unwrap_err();
        assert!(err.contains("应用目录"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn launch_urls_are_only_given_for_running_apps() {
        let tmp = tempfile::tempdir().unwrap();
        let svc =
            AppService::start_with(&tmp.path().join("bytehost"), GatewayConfig { port: 0 }).await;
        install(&svc, write_app(&tmp.path().join("src/a"), "alpha", "A")).await;
        // 已安装但没运行
        assert!(
            svc.handle(AppRequest::LaunchUrl { id: id("alpha") })
                .await
                .is_err()
        );
        // 没安装
        assert!(
            svc.handle(AppRequest::LaunchUrl { id: id("ghost") })
                .await
                .is_err()
        );
        svc.handle(AppRequest::Start { id: id("alpha") })
            .await
            .unwrap();
        let url = running_url(
            svc.handle(AppRequest::LaunchUrl { id: id("alpha") })
                .await
                .unwrap(),
        );
        assert!(
            url.starts_with("http://alpha.localhost:") && url.contains("bh_token="),
            "{url}"
        );
        assert_eq!(fetch(&url, "alpha"), (200, "A".to_string()));
        svc.shutdown().await;
    }

    /// 应用跟随 dozerd:dozerd 停止则应用停止(站点撤下),但用户想要运行的应用在下次启动时自动恢复。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn apps_wanted_running_come_back_after_a_dozerd_restart_and_stopped_ones_stay_stopped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("bytehost");
        let first = AppService::start_with(&root, GatewayConfig { port: 0 }).await;
        install(
            &first,
            write_app(&tmp.path().join("src/run"), "runner", "R"),
        )
        .await;
        install(
            &first,
            write_app(&tmp.path().join("src/idle"), "idler", "I"),
        )
        .await;
        first
            .handle(AppRequest::Start { id: id("runner") })
            .await
            .unwrap();
        first
            .handle(AppRequest::Start { id: id("idler") })
            .await
            .unwrap();
        first
            .handle(AppRequest::Stop { id: id("idler") })
            .await
            .unwrap();
        let old_url = running_url(
            first
                .handle(AppRequest::LaunchUrl { id: id("runner") })
                .await
                .unwrap(),
        );
        assert_eq!(fetch(&old_url, "runner").0, 200);

        first.shutdown().await;
        assert!(first.gateway_stopped(), "dozerd 停了,gateway 也停了");

        let second = AppService::start_with(&root, GatewayConfig { port: 0 }).await;
        let AppReply::Apps { apps } = second.handle(AppRequest::List).await.unwrap() else {
            panic!()
        };
        let runner = apps.iter().find(|a| a.id == id("runner")).unwrap();
        let idler = apps.iter().find(|a| a.id == id("idler")).unwrap();
        assert!(
            runner.url.is_some(),
            "desired=Running 的应用被自动恢复: {runner:?}"
        );
        assert!(idler.url.is_none(), "desired=Stopped 的保持停止: {idler:?}");
        let url = running_url(
            second
                .handle(AppRequest::LaunchUrl { id: id("runner") })
                .await
                .unwrap(),
        );
        assert_eq!(fetch(&url, "runner"), (200, "R".to_string()));
        second.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn uninstalling_removes_the_app_from_the_list_and_probes_name_the_three_runtimes() {
        let tmp = tempfile::tempdir().unwrap();
        let svc =
            AppService::start_with(&tmp.path().join("bytehost"), GatewayConfig { port: 0 }).await;
        install(&svc, write_app(&tmp.path().join("src/a"), "alpha", "A")).await;
        svc.handle(AppRequest::Uninstall {
            id: id("alpha"),
            mode: UninstallMode::ProgramAndData,
        })
        .await
        .unwrap();
        let AppReply::Apps { apps } = svc.handle(AppRequest::List).await.unwrap() else {
            panic!()
        };
        assert!(apps.is_empty());
        let AppReply::Runtimes { runtimes } = svc.handle(AppRequest::ProbeRuntimes).await.unwrap()
        else {
            panic!()
        };
        let names: Vec<_> = runtimes.iter().map(|r| r.runtime.as_str()).collect();
        assert_eq!(names, ["docker", "node", "python"]);
        svc.shutdown().await;
    }

    /// 端口文件坏了:服务不可用并说明原因,**坏文件原样保留**(悄悄重选一个端口会让所有应用丢本地数据)。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_corrupt_gateway_port_file_makes_the_service_unavailable_and_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("bytehost");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("gateway.json"), "{not json").unwrap();
        let svc = AppService::start(&root).await;
        let err = svc.handle(AppRequest::List).await.unwrap_err();
        assert!(
            err.contains("应用宿主不可用") && err.contains("gateway.json"),
            "{err}"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("gateway.json")).unwrap(),
            "{not json"
        );
    }

    /// `start` 用持久化的端口:停了再起,端口不变(origin 含端口,端口变了所有应用的本地存储都会丢)。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_uses_the_persisted_port_across_restarts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("bytehost");
        std::fs::create_dir_all(&root).unwrap();
        let free = std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let free = if free < 1024 { 20000 } else { free };
        std::fs::write(root.join("gateway.json"), format!("{{\"port\": {free}}}")).unwrap();

        install_and_run_on(&root, tmp.path(), free).await;
        install_and_run_on(&root, tmp.path(), free).await;
    }

    async fn install_and_run_on(root: &Path, scratch: &Path, expected_port: u16) {
        let svc = AppService::start(root).await;
        let AppReply::Apps { .. } = svc.handle(AppRequest::List).await.unwrap() else {
            panic!()
        };
        install_if_missing(&svc, scratch).await;
        let url = match svc
            .handle(AppRequest::Start { id: id("alpha") })
            .await
            .unwrap()
        {
            AppReply::Started { url } => url,
            other => panic!("{other:?}"),
        };
        assert!(url.contains(&format!(":{expected_port}/")), "{url}");
        svc.shutdown().await;
    }

    async fn install_if_missing(svc: &AppService, scratch: &Path) {
        let AppReply::Apps { apps } = svc.handle(AppRequest::List).await.unwrap() else {
            panic!()
        };
        if apps.is_empty() {
            install(svc, write_app(&scratch.join("src/a"), "alpha", "A")).await;
        }
    }
}
```

`$SCRATCH/a2/app_requests.rs`:

```rust
//! 应用宿主请求经 UDS 到达 dozerd 的 `AppService`:走真实的 socket 协议与 `dozer-client`。

use bytehost_apps::gateway::GatewayConfig;
use bytehost_apps::id::AppId;
use bytehost_apps::plan::{Approval, Provenance, TrustLevel};
use bytehost_apps::proto::AppSource;
use bytehost_apps::registry::UninstallMode;
use dozer_client::Client;
use dozerd::app_service::AppService;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

struct CleanupGuard(PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct TestDaemon {
    client: Client,
    _cleanup: CleanupGuard,
    _task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

async fn start_daemon(apps: Arc<AppService>) -> TestDaemon {
    let sock = PathBuf::from(format!("/tmp/dz-apps-{}.sock", uuid::Uuid::new_v4()));
    let db = PathBuf::from(format!("/tmp/dz-apps-{}.db", uuid::Uuid::new_v4()));
    let stores = dozerd::server::Stores {
        registry: Arc::new(dozerd::registry::SessionRegistry::new()),
        projects: Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap()),
        bookmarks: Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap()),
        code_health: Arc::new(dozerd::code_health::CodeHealthStore::new(&db).unwrap()),
        transcripts: Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap()),
        session_summaries: Arc::new(
            dozerd::session_summary::SessionSummaryStore::open(&db).unwrap(),
        ),
        summary_jobs: Arc::new(dozerd::summary_jobs::SummaryJobStore::open(&db).unwrap()),
        backfill_registry: Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new()),
        todos: Arc::new(dozerd::todo::TodoStore::new(&db).unwrap()),
        categories: Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap()),
        memories: Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap()),
        file_edit_history: Arc::new(
            dozerd::file_edit_history::FileEditHistoryStore::new(&db).unwrap(),
        ),
        groups: dozerd::group_service::GroupService::for_tests(),
        apps,
    };
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    let task = tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            stores,
            dozerd::task_poller::new_in_flight(),
        )
        .await
    });
    for _ in 0..100 {
        if tokio::net::UnixStream::connect(&sock).await.is_ok() {
            return TestDaemon {
                client: Client::new(sock.clone()),
                _cleanup: CleanupGuard(sock),
                _task: task,
            };
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等待测试 dozerd 监听超时");
}

fn write_app(dir: &Path, id: &str, body: &str) -> AppSource {
    std::fs::create_dir_all(dir.join("web")).unwrap();
    std::fs::write(
        dir.join("manifest.toml"),
        format!(
            "schema_version = 1\nmin_host_version = \"0.1.0\"\nid = \"{id}\"\nname = \"{id} app\"\nversion = \"1.0.0\"\n\n\
             [presentation]\nentrypoint = \"main\"\n\n[entrypoints.main]\ntype = \"web\"\npath = \"/\"\n\n\
             [runtime]\nkind = \"static_web\"\nsource = \"web/\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("web/index.html"), body).unwrap();
    AppSource::LocalDir {
        path: dir.to_path_buf(),
    }
}

fn fetch(launch_url: &str, id: &str) -> (u16, String) {
    let rest = launch_url.strip_prefix("http://").unwrap();
    let (authority, query) = rest.split_once('/').unwrap();
    let port: u16 = authority.rsplit_once(':').unwrap().1.parse().unwrap();
    let token = query.split("bh_token=").nth(1).unwrap();
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(
        s,
        "GET / HTTP/1.1\r\nHost: {id}.localhost:{port}\r\nCookie: bh_session={token}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    let status = raw.split_whitespace().nth(1).unwrap().parse().unwrap();
    (
        status,
        raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string(),
    )
}

fn id(s: &str) -> AppId {
    AppId::new(s).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_whole_install_run_stop_uninstall_cycle_works_over_the_socket() {
    let tmp = tempfile::tempdir().unwrap();
    let apps =
        AppService::start_with(&tmp.path().join("bytehost"), GatewayConfig { port: 0 }).await;
    let d = start_daemon(apps).await;
    let c = &d.client;
    let source = write_app(&tmp.path().join("src/a"), "excalidraw", "<h1>hello</h1>");

    let plan = c
        .app_plan(source.clone(), Provenance::Local, TrustLevel::Trusted)
        .await
        .unwrap();
    assert_eq!(plan.app_id, id("excalidraw"));
    assert_eq!(plan.runtime_kind, "static_web");
    assert!(c.app_list().await.unwrap().is_empty(), "出计划不安装");
    c.app_install(
        plan.approve(Approval {
            approver: "test".into(),
            approved_ms: 1,
        }),
        source,
    )
    .await
    .unwrap();
    let listed = c.app_list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "excalidraw app");
    assert!(listed[0].url.is_none());

    let url = c.app_start(id("excalidraw")).await.unwrap();
    assert!(
        url.starts_with("http://excalidraw.localhost:") && !url.contains("bh_token"),
        "{url}"
    );
    let launch = c.app_launch_url(id("excalidraw")).await.unwrap();
    assert_eq!(
        fetch(&launch, "excalidraw"),
        (200, "<h1>hello</h1>".to_string())
    );

    c.app_stop(id("excalidraw")).await.unwrap();
    assert!(
        c.app_launch_url(id("excalidraw")).await.is_err(),
        "停止后不再发带令牌的地址"
    );
    c.app_uninstall(id("excalidraw"), UninstallMode::ProgramAndData)
        .await
        .unwrap();
    assert!(c.app_list().await.unwrap().is_empty());

    let err = c.app_start(id("excalidraw")).await.unwrap_err().to_string();
    assert!(err.contains("没有安装"), "{err}");
    let runtimes = c.app_probe_runtimes().await.unwrap();
    assert_eq!(runtimes.len(), 3);
}

/// 应用宿主不可用(例如端口被占)不能影响会话等其他功能:其他请求照常工作,应用请求带着原因失败。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unavailable_app_host_does_not_break_the_rest_of_the_daemon() {
    let d = start_daemon(AppService::unavailable("gateway 端口 12345 已被占用")).await;
    assert!(d.client.list().await.unwrap().is_empty(), "会话列表照常");
    let err = d.client.app_list().await.unwrap_err().to_string();
    assert!(err.contains("12345") && err.contains("占用"), "{err}");
}

/// `Shutdown` 请求:应用跟随 dozerd 停止(gateway 关闭、站点撤下),但用户想要运行的意愿(desired)保留。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shutdown_request_takes_the_apps_down_but_keeps_what_the_user_wanted() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("bytehost");
    let apps = AppService::start_with(&root, GatewayConfig { port: 0 }).await;
    let d = start_daemon(apps.clone()).await;
    let c = &d.client;
    let source = write_app(&tmp.path().join("src/a"), "excalidraw", "hi");
    let plan = c
        .app_plan(source.clone(), Provenance::Local, TrustLevel::Trusted)
        .await
        .unwrap();
    c.app_install(
        plan.approve(Approval {
            approver: "t".into(),
            approved_ms: 1,
        }),
        source,
    )
    .await
    .unwrap();
    let url = c.app_start(id("excalidraw")).await.unwrap();
    let port: u16 = url
        .split(':')
        .nth(2)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());

    c.shutdown_daemon().await.unwrap();

    assert!(apps.gateway_stopped(), "dozerd 停了,gateway 也停了");
    let state: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("apps/excalidraw/state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        state["desired"], "running",
        "用户想要运行的意愿保留,下次启动自动恢复"
    );
    assert_eq!(state["observed"]["state"], "stopped");
}
```

`$SCRATCH/a2/tests_block_manager.rs`:

```rust
    /// supervisor 退出:站点撤下、观察态落成 Stopped,但 `desired` 保持 Running,下次 `reconcile` 把它们拉起来。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn suspending_keeps_what_the_user_wanted_so_the_next_start_restores_it() {
        let rig = rig().await;
        let (a, b) = (id("alpha"), id("beta"));
        rig.install(&write_app(&rig.src_dir("a"), "alpha", "1.0.0", "", "A"))
            .unwrap();
        rig.install(&write_app(&rig.src_dir("b"), "beta", "1.0.0", "", "B"))
            .unwrap();
        rig.manager.start(&a).unwrap();
        rig.manager.start(&b).unwrap();
        rig.manager.stop(&b).unwrap(); // beta:用户想要停止

        assert!(rig.manager.suspend_all().is_empty());
        assert!(!rig.gateway.has_site(&a), "站点已撤下");
        let after = rig.manager.list().unwrap();
        let alpha = after.iter().find(|x| x.id == a).unwrap();
        assert_eq!(
            (alpha.desired, alpha.observed.clone()),
            (DesiredState::Running, ObservedState::Stopped)
        );
        let beta = after.iter().find(|x| x.id == b).unwrap();
        assert_eq!(
            (beta.desired, beta.observed.clone()),
            (DesiredState::Stopped, ObservedState::Stopped)
        );

        assert!(rig.manager.reconcile().is_empty());
        assert!(rig.gateway.has_site(&a), "desired=Running 的被重新拉起");
        assert!(!rig.gateway.has_site(&b));
    }
```

`$SCRATCH/a2/tests_block_gateway.rs`:

```rust
    /// `stop` 只要 `&self`、可以重复调用;完成之后 `is_stopped()` 为真。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_is_idempotent_and_marks_the_gateway_stopped() {
        let gw = Gateway::start(GatewayConfig { port: 0 }).await.unwrap();
        assert!(!gw.is_stopped());
        gw.stop().await;
        assert!(gw.is_stopped());
        gw.stop().await;
        assert!(gw.is_stopped());
        gw.shutdown().await;
    }
```

`$SCRATCH/a2/tests_block_core.rs`:

```rust
    #[test]
    fn app_host_requests_and_replies_roundtrip_through_the_line_codec() {
        use bytehost_apps::id::AppId;
        use bytehost_apps::plan::{Provenance, TrustLevel};
        use bytehost_apps::proto::{
            AppReply, AppRequest, AppSource, RuntimeAvailability, RuntimeProbe,
        };

        let requests = [
            AppRequest::List,
            AppRequest::ProbeRuntimes,
            AppRequest::Start {
                id: AppId::new("excalidraw").unwrap(),
            },
            AppRequest::Plan {
                source: AppSource::LocalDir {
                    path: "/tmp/x".into(),
                },
                provenance: Provenance::Local,
                trust: TrustLevel::Trusted,
            },
        ];
        for request in requests {
            let req = Request::App { request };
            let line = encode_line(&req);
            assert!(line.contains("\"type\":\"app\""), "{line}");
            assert_eq!(decode_line::<Request>(&line).unwrap(), req);
        }
        let reply = Reply::App {
            reply: AppReply::Runtimes {
                runtimes: vec![RuntimeProbe {
                    runtime: "docker".into(),
                    availability: RuntimeAvailability::NotInstalled,
                }],
            },
        };
        assert_eq!(decode_line::<Reply>(&encode_line(&reply)).unwrap(), reply);
        // 未知的 op 在协议层就被拒绝
        assert!(
            decode_line::<Request>("{\"type\":\"app\",\"request\":{\"op\":\"format_disk\"}}")
                .is_err()
        );
    }
```

`$SCRATCH/a2_tests.py`:

```python
#!/usr/bin/env python3
"""A2 一次性脚本(步骤 3,RED):写入全部测试。
- 新文件 `proto.rs`、`app_service.rs` 先只含测试(实现在步骤 5 补上),`app_requests.rs` 整个是测试;
- 已有的 `manager.rs`、`gateway/mod.rs`、`dozer-core` 的 `protocol.rs` 各追加一个测试;
- 声明两个新模块。`SP` 是 tests_*.rs 等所在目录。**不提交。**"""
import os, shutil

SP = os.environ["SP"]


def read(p):
    return open(p).read()


def write(p, s):
    open(p, "w").write(s)


def rd(name):
    return read(os.path.join(SP, name))


def edit(path, a, b):
    s = read(path)
    assert a in s, (path, a[:60])
    write(path, s.replace(a, b, 1))


def append_block(path, name):
    s = read(path).rstrip("\n")
    assert s.endswith("\n}"), path
    write(path, s[:-1].rstrip("\n") + "\n\n" + rd(name).rstrip("\n") + "\n}\n")


write("crates/bytehost-apps/src/proto.rs", rd("tests_proto.rs").lstrip("\n"))
write("crates/dozerd/src/app_service.rs", rd("tests_app_service.rs").lstrip("\n"))
shutil.copy(os.path.join(SP, "app_requests.rs"), "crates/dozerd/tests/app_requests.rs")
edit("crates/bytehost-apps/src/lib.rs", "pub mod plan;\n", "pub mod plan;\npub mod proto;\n")
edit("crates/dozerd/src/lib.rs", "pub mod agent_context;\n", "pub mod agent_context;\npub mod app_service;\n")
append_block("crates/bytehost-apps/src/manager.rs", "tests_block_manager.rs")
append_block("crates/bytehost-apps/src/gateway/mod.rs", "tests_block_gateway.rs")
anchor = "mod tests {\n    use super::*;\n"
s = read("crates/dozer-core/src/protocol.rs")
i = s.index(anchor) + len(anchor)
write("crates/dozer-core/src/protocol.rs", s[:i] + "\n" + rd("tests_block_core.rs").rstrip("\n") + "\n" + s[i:])
print("ok")
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a2 && export PYTHONDONTWRITEBYTECODE=1
SP=$SCRATCH/a2 python3 $SCRATCH/a2_tests.py
cargo test -p bytehost-apps -p dozerd -p dozer-core --no-run 2>&1 | grep -E "^error" | sort | uniq -c | head -8
```
Expected: 脚本输出 `ok`;编译失败(`bytehost-apps (lib test)`、`dozer-core (lib test)` 等),报错是 `cannot find …`(如 `Serialize`/`Deserialize`、`AppSummary`、`RuntimeProbe`、`AppRequest`、`stop`/`is_stopped`、`suspend_all`、`bytehost_apps`)——RED,测试引用的实现都还不存在。

- [ ] **Step 4: 写实现源文件与脚本**

把下面各段写成 `$SCRATCH/a2/impl_*.rs`,再写实现脚本 `$SCRATCH/a2_impl.py`(带引号 heredoc):

`$SCRATCH/a2/impl_proto.rs`:

```rust
//! GUI ↔ supervisor 的线上类型(默认 feature,只依赖 serde):请求、应答,以及它们带着的列表项/探测结果。
//! 宿主产品把 [`AppRequest`]/[`AppReply`] 原样塞进自己的 socket 协议里(Dozer 见 `dozer-core::protocol` 的
//! `Request::App`/`Reply::App`),Digger 将来用同一组类型走它自己的 socket。**本 crate 不知道 socket 长什么样。**

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::id::{AppId, Version};
use crate::plan::{ApprovedInstallPlan, InstallPlan, Provenance, TrustLevel};
use crate::registry::UninstallMode;
use crate::state::{DesiredState, ObservedState};

/// 应用从哪来。一期只有本地目录(目录里要有 `manifest.toml`);压缩包/仓库以后再加。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppSource {
    LocalDir { path: PathBuf },
}

/// 列表里的一项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSummary {
    pub id: AppId,
    pub name: String,
    pub version: Version,
    pub desired: DesiredState,
    pub observed: ObservedState,
    /// 运行中才有:不含令牌的站点地址。
    pub url: Option<String>,
}

/// 一个外部运行时的可用性(分层:没装 / 装了但当前不可用 / 可用)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProbe {
    /// `docker` / `node` / `python`。
    pub runtime: String,
    #[serde(flatten)]
    pub availability: RuntimeAvailability,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AppRequest {
    /// 已安装的应用(含状态与站点地址)。
    List,
    /// 出安装计划(不安装任何东西)。
    Plan {
        source: AppSource,
        provenance: Provenance,
        trust: TrustLevel,
    },
    /// 安装一份**已批准**的计划;服务端会对 staging 副本重新计算并核对,不信任这里带来的内容。
    Install {
        approved: Box<ApprovedInstallPlan>,
        source: AppSource,
    },
    Start {
        id: AppId,
    },
    Stop {
        id: AppId,
    },
    Uninstall {
        id: AppId,
        mode: UninstallMode,
    },
    /// 首次导航用的地址(含一次性换 Cookie 的令牌,**秘密**)。
    LaunchUrl {
        id: AppId,
    },
    /// 探测 docker/node/python 的可用性(Settings 展示用)。
    ProbeRuntimes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum AppReply {
    Apps {
        apps: Vec<AppSummary>,
    },
    Plan {
        plan: Box<InstallPlan>,
    },
    /// 成功且没有内容(安装、停止、卸载)。
    Done,
    /// 已启动;`url` 不含令牌。
    Started {
        url: String,
    },
    LaunchUrl {
        url: String,
    },
    Runtimes {
        runtimes: Vec<RuntimeProbe>,
    },
}
```

`$SCRATCH/a2/impl_app_service.rs`:

```rust
//! 应用宿主在 dozerd 里的服务:持有 `AppManager` 与 gateway,把线上的 [`AppRequest`] 翻译成对它们的调用。
//!
//! 设计要点(规格 §6.1,用户已定:**应用跟随 dozerd,dozerd 停止则应用停止**):
//! - **启动永不失败**:gateway 端口被占用、应用目录不可用等问题只会让这个服务变成"不可用"(每个请求回带原因的
//!   错误),不会拖垮 dozerd 的会话功能;
//! - 启动时 `reconcile`(把上次随 dozerd 一起停掉的应用,按 `desired = Running` 重新拉起);
//! - 退出时 `shutdown`:撤下站点、观察态落成 `Stopped`,**保留 `desired`**,下次启动自动恢复;
//! - `AppManager` 做阻塞文件 I/O 并持有 `std::sync::Mutex`,所以每次调用都经 `spawn_blocking`。

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use bytehost_apps::HOST_VERSION;
use bytehost_apps::gateway::{Gateway, GatewayConfig};
use bytehost_apps::manager::{AppManager, ManagerError};
use bytehost_apps::port::load_or_choose_port;
use bytehost_apps::proto::{AppReply, AppRequest, RuntimeProbe};
use bytehost_apps::runtime::{SystemRunner, probe_all};

dozer_core::scope!(LOG, module, "apps");

enum State {
    Ready {
        manager: Arc<AppManager>,
        gateway: Arc<Gateway>,
    },
    Unavailable(String),
}

pub struct AppService {
    state: State,
}

impl AppService {
    /// 一个永远回错误的服务(测试里不需要应用宿主时用,也是启动失败时的形态)。
    pub fn unavailable(reason: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            state: State::Unavailable(reason.into()),
        })
    }

    /// 用**持久化的端口**启动(首次启动随机选一个并写进 `<root>/gateway.json`)。永不失败。
    pub async fn start(root: &Path) -> Arc<Self> {
        match load_or_choose_port(root) {
            Ok(port) => Self::start_with(root, GatewayConfig { port }).await,
            Err(e) => {
                dozer_core::log_error!(LOG, error = %e, "读取 gateway 端口失败,应用宿主不可用");
                Self::unavailable(format!("应用宿主不可用:{e}"))
            }
        }
    }

    /// 用给定的 gateway 配置启动(测试用 `port: 0`)。永不失败。
    pub async fn start_with(root: &Path, config: GatewayConfig) -> Arc<Self> {
        let gateway = match Gateway::start(config).await {
            Ok(g) => Arc::new(g),
            Err(e) => {
                dozer_core::log_error!(LOG, error = %e, "gateway 启动失败,应用宿主不可用");
                return Self::unavailable(format!("应用宿主不可用:{e}"));
            }
        };
        let manager = match AppManager::new(root, HOST_VERSION, gateway.clone()) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                gateway.stop().await;
                dozer_core::log_error!(LOG, error = %e, "应用目录不可用");
                return Self::unavailable(format!("应用宿主不可用:应用目录打不开:{e}"));
            }
        };
        spawn_event_logger(&manager);
        let for_reconcile = manager.clone();
        let failures = tokio::task::spawn_blocking(move || for_reconcile.reconcile())
            .await
            .unwrap_or_default();
        for (app, error) in failures {
            dozer_core::log_warn!(LOG, app = %app, error = %error, "启动对账:应用恢复失败");
        }
        dozer_core::log_info!(LOG, port = gateway.port(), "应用宿主已启动");
        Arc::new(Self {
            state: State::Ready { manager, gateway },
        })
    }

    pub async fn handle(&self, request: AppRequest) -> Result<AppReply, String> {
        let State::Ready { manager, .. } = &self.state else {
            let State::Unavailable(reason) = &self.state else {
                unreachable!()
            };
            return Err(reason.clone());
        };
        match request {
            AppRequest::List => blocking(manager, |m| m.list())
                .await
                .map(|apps| AppReply::Apps { apps }),
            AppRequest::Plan {
                source,
                provenance,
                trust,
            } => blocking(manager, move |m| m.install_plan(&source, provenance, trust))
                .await
                .map(|plan| AppReply::Plan {
                    plan: Box::new(plan),
                }),
            AppRequest::Install { approved, source } => {
                blocking(manager, move |m| m.install(&approved, &source, now_ms()))
                    .await
                    .map(|()| AppReply::Done)
            }
            AppRequest::Start { id } => blocking(manager, move |m| m.start(&id))
                .await
                .map(|url| AppReply::Started { url }),
            AppRequest::Stop { id } => blocking(manager, move |m| m.stop(&id))
                .await
                .map(|()| AppReply::Done),
            AppRequest::Uninstall { id, mode } => {
                blocking(manager, move |m| m.uninstall(&id, mode))
                    .await
                    .map(|()| AppReply::Done)
            }
            AppRequest::LaunchUrl { id } => {
                // 只给正在运行的应用发带令牌的地址;没运行的应用打开只会是 404
                blocking(manager, move |m| {
                    let running = m.list()?.into_iter().find(|a| a.id == id);
                    match running {
                        Some(a) if a.url.is_some() => Ok(m.launch_url(&id)),
                        Some(_) => Err(ManagerError::BadState {
                            app: id,
                            state: bytehost_apps::state::ObservedState::Stopped,
                        }),
                        None => Err(ManagerError::NotInstalled(id)),
                    }
                })
                .await
                .map(|url| AppReply::LaunchUrl { url })
            }
            AppRequest::ProbeRuntimes => {
                let probes = tokio::task::spawn_blocking(|| probe_all(&SystemRunner::default()))
                    .await
                    .map_err(|e| format!("探测任务失败: {e}"))?;
                Ok(AppReply::Runtimes {
                    runtimes: probes
                        .into_iter()
                        .map(|(runtime, availability)| RuntimeProbe {
                            runtime: runtime.to_string(),
                            availability,
                        })
                        .collect(),
                })
            }
        }
    }

    /// gateway 已经停了(或者根本没起来)。测试用它判断"dozerd 停了应用也停了"——不靠再连一次端口,
    /// 端口一释放就可能被别的程序立刻重新占用。
    pub fn gateway_stopped(&self) -> bool {
        match &self.state {
            State::Ready { gateway, .. } => gateway.is_stopped(),
            State::Unavailable(_) => true,
        }
    }

    /// dozerd 退出前调用(幂等):撤下站点、观察态落成 `Stopped`、停 gateway;`desired` 保留。
    pub async fn shutdown(&self) {
        let State::Ready { manager, gateway } = &self.state else {
            return;
        };
        let m = manager.clone();
        let failures = tokio::task::spawn_blocking(move || m.suspend_all())
            .await
            .unwrap_or_default();
        for (app, error) in failures {
            dozer_core::log_warn!(LOG, app = %app, error = %error, "退出时收尾失败");
        }
        gateway.stop().await;
    }
}

async fn blocking<T, F>(manager: &Arc<AppManager>, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&AppManager) -> Result<T, ManagerError> + Send + 'static,
{
    let manager = manager.clone();
    tokio::task::spawn_blocking(move || f(&manager))
        .await
        .map_err(|e| format!("应用宿主任务失败: {e}"))?
        .map_err(|e| e.to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 把应用事件写进日志(事件里没有令牌:`EndpointChanged` 带的是不含令牌的站点地址)。
fn spawn_event_logger(manager: &Arc<AppManager>) {
    let mut rx = manager.events();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    dozer_core::log_info!(LOG, app = %event.app(), event = ?event, "应用事件")
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
```

`$SCRATCH/a2_impl.py`:

```python
#!/usr/bin/env python3
"""A2 一次性实现脚本(步骤 5):把应用宿主接进 dozer-core / dozerd / dozer-client。**不提交。** 在仓库根运行,
每个替换点先 assert。新文件 `proto.rs` 与 `app_service.rs` 由 `impl_*.rs` + `tests_*.rs` 拼成。`SP` 同步骤 3。"""
import glob, os, re

SP = os.environ["SP"]
B = "crates/bytehost-apps/src/"


def read(p):
    return open(p).read()


def write(p, s):
    open(p, "w").write(s)


def rd(name):
    return read(os.path.join(SP, name))


def edit(path, pairs, count=1):
    s = read(path)
    for a, b in pairs:
        assert a in s, (path, a[:70])
        s = s.replace(a, b) if count == 0 else s.replace(a, b, count)
    write(path, s)


# ---- bytehost-apps ----
write(B + "proto.rs", rd("impl_proto.rs") + rd("tests_proto.rs"))
s = read(B + "lib.rs").rstrip("\n") + """

/// 应用宿主的 API 版本:manifest 的 `min_host_version` 比较的是它,**不是**宿主产品(Dozer/Digger)自己的版本号。
pub const HOST_VERSION: id::Version = id::Version::new(0, 1, 0);
"""
write(B + "lib.rs", s)
edit(B + "registry.rs", [("#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum UninstallMode {", "#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]\n#[serde(rename_all = \"snake_case\")]\npub enum UninstallMode {")])

# manager:类型搬进 proto(re-export),`LocalDir` 变成结构体变体,新增 suspend_all
s = read(B + "manager.rs")
i = s.index("/// 应用从哪来。一期只有本地目录"); j = s.index("#[derive(Debug)]\npub enum ManagerError")
s = s[:i] + "pub use crate::proto::{AppSource, AppSummary};\n\n" + s[j:]
i = s.index("/// 列表里的一项。"); j = s.index("pub struct AppManager {")
s = s[:i] + s[j:]
s = s.replace("let AppSource::LocalDir(dir) = source;", "let AppSource::LocalDir { path: dir } = source;")
s = s.replace("AppSource::LocalDir(dir.to_path_buf())", "AppSource::LocalDir { path: dir.to_path_buf() }")
s = s.replace("AppSource::LocalDir(dir)", "AppSource::LocalDir { path: dir }")
s = s.replace("AppSource::LocalDir(unsupported)", "AppSource::LocalDir { path: unsupported }")
a = "    /// supervisor 启动时调用:把持久化的观察态修正为现实"
assert a in s
s = s.replace(a, """    /// supervisor 退出前调用:撤下所有站点,把运行中的应用的观察态落成 `Stopped`,但**保留 `desired`**——
    /// 下次启动时 `reconcile` 会按 `desired = Running` 把它们重新拉起("应用跟随 dozerd")。返回出错的应用。
    pub fn suspend_all(&self) -> Vec<(AppId, ManagerError)> {
        let _guard = self.lock.lock().expect("manager 锁");
        let mut failures = Vec::new();
        let listing = match self.registry.list() {
            Ok(l) => l,
            Err(e) => return vec![(AppId::new("unknown").expect("合法"), ManagerError::Io(e))],
        };
        for mut record in listing.apps {
            self.gateway.remove_site(&record.id);
            if matches!(record.observed, ObservedState::Running)
                && let Err(e) = self.set_observed(&mut record, ObservedState::Stopped)
            {
                failures.push((record.id.clone(), e));
            }
        }
        failures
    }

""" + a, 1)
write(B + "manager.rs", s)

# probe:RuntimeAvailability 搬进 proto
s = read(B + "runtime/probe.rs")
i = s.index("#[derive(Debug, Clone, PartialEq, Eq)]\npub enum RuntimeAvailability {")
j = s.index("#[derive(Debug, Clone, PartialEq, Eq)]\npub struct CommandOutput")
s = s[:i] + "pub use crate::proto::RuntimeAvailability;\n\n" + s[j:]
write(B + "runtime/probe.rs", s)

# gateway:`stop(&self)`(幂等)、`is_stopped`
s = read(B + "gateway/mod.rs")
for a, b in [
    ("    task: tokio::task::JoinHandle<()>,\n}", "    task: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,\n}"),
    ("        Ok(Self {\n            state,\n            shutdown,\n            task,\n        })", "        Ok(Self {\n            state,\n            shutdown,\n            task: std::sync::Mutex::new(Some(task)),\n        })"),
    ("    pub fn has_site(&self, id: &AppId) -> bool {", """    /// `stop`/`shutdown` 已经完成(accept 循环已退出)。测试与上层用它判断"这个 gateway 确实停了",
    /// 而不是靠"再连一次端口"——端口一释放就可能被别的程序(或别的测试)立刻重新占用,连接探测会误判。
    pub fn is_stopped(&self) -> bool {
        self.task.lock().expect("task 锁").is_none()
    }

    pub fn has_site(&self, id: &AppId) -> bool {"""),
]:
    assert a in s, a[:60]
    s = s.replace(a, b, 1)
i = s.index("    pub async fn shutdown(self) {")
j = s.index("\n    }\n", i) + len("\n    }\n")
s = s[:i] + """    /// 停止接受新连接并等 accept 循环退出。**幂等**,且只要 `&self`——`AppManager` 与 dozerd 都持有 `Arc<Gateway>`,
    /// 没法交出所有权调用 [`Gateway::shutdown`]。已建立的连接不会被主动掐断。
    pub async fn stop(&self) {
        // notify_one 会存一个许可:即使 accept 循环还没跑到 `notified()`,稍后也能收到(notify_waiters 会丢)
        self.shutdown.notify_one();
        let task = self.task.lock().expect("task 锁").take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    pub async fn shutdown(self) {
        self.stop().await;
    }
""" + s[j:]
write(B + "gateway/mod.rs", s)

# ---- dozer-core ----
edit("crates/dozer-core/Cargo.toml", [("anyhow.workspace = true\n", "anyhow.workspace = true\n# 应用宿主的线上类型(默认 feature:只有 serde 依赖,`dozer-hook`/`dozer-mcp` 的依赖闭包不会因此多出新 crate)\nbytehost-apps = { path = \"../bytehost-apps\" }\n")])
edit("crates/dozer-core/src/protocol.rs", [
    ("pub enum Request {\n    ListSessions,", """pub enum Request {
    /// bytehost 应用宿主的请求(安装/启动/停止/列表……),原样交给 dozerd 里的 `AppService`。
    /// 用结构体变体包一层,避免"内部带标签的枚举套内部带标签的枚举"。
    App {
        request: bytehost_apps::proto::AppRequest,
    },
    ListSessions,"""),
    ("pub enum Reply {\n    Sessions {", """pub enum Reply {
    /// 应用宿主请求的应答。失败走通用的 `Error`。
    App {
        reply: bytehost_apps::proto::AppReply,
    },
    Sessions {"""),
])

# ---- dozerd ----
edit("crates/dozerd/Cargo.toml", [("dozer-core = { path = \"../dozer-core\", features = [\"logging\"] }\n", "dozer-core = { path = \"../dozer-core\", features = [\"logging\"] }\n# 应用宿主:`server` feature = gateway + AppManager(只有 dozerd 打开)\nbytehost-apps = { path = \"../bytehost-apps\", features = [\"server\"] }\n")])
write("crates/dozerd/src/app_service.rs", rd("impl_app_service.rs") + rd("tests_app_service.rs"))
s = read("crates/dozerd/src/server.rs")
for a, b, cnt in [
    ("    pub groups: std::sync::Arc<crate::group_service::GroupService>,\n}", "    pub groups: std::sync::Arc<crate::group_service::GroupService>,\n    pub apps: std::sync::Arc<crate::app_service::AppService>,\n}", 1),
    ("        file_edit_history,\n        groups,\n    } = stores;", "        file_edit_history,\n        groups,\n        apps,\n    } = stores;", 0),
    ("        file_edit_history,\n        groups,\n    };\n    loop {", "        file_edit_history,\n        groups,\n        apps,\n    };\n    loop {", 1),
    ("                                drain_all_sessions(registry.clone(), summary_service.clone());\n", "                                drain_all_sessions(registry.clone(), summary_service.clone());\n                                // 应用跟随 dozerd:撤下站点、观察态落成 Stopped(desired 保留,下次启动自动恢复)\n                                apps.shutdown().await;\n", 1),
    ("                        Request::ListSessions => Reply::Sessions { sessions: registry.list() },", """                        Request::App { request } => match apps.handle(request).await {
                            Ok(reply) => Reply::App { reply },
                            Err(message) => Reply::Error { message },
                        },
                        Request::ListSessions => Reply::Sessions { sessions: registry.list() },""", 1),
    ("                    file_edit_history,\n                    groups,\n                },\n                crate::task_poller::new_in_flight(),", "                    file_edit_history,\n                    groups,\n                    apps: crate::app_service::AppService::unavailable(\"test\"),\n                },\n                crate::task_poller::new_in_flight(),", 1),
]:
    assert a in s, a[:60]
    s = s.replace(a, b) if cnt == 0 else s.replace(a, b, cnt)
write("crates/dozerd/src/server.rs", s)
edit("crates/dozerd/src/main.rs", [
    ("    let in_flight = dozerd::task_poller::new_in_flight();\n", "    // 应用宿主:启动永不失败(端口被占用等只会让它\"不可用\",不影响会话功能);启动时对账,退出时收尾\n    let apps = dozerd::app_service::AppService::start(&dozer_core::paths::state_dir().join(\"bytehost\")).await;\n    let in_flight = dozerd::task_poller::new_in_flight();\n"),
    ("            file_edit_history,\n            groups,\n        },\n        in_flight,", "            file_edit_history,\n            groups,\n            apps: apps.clone(),\n        },\n        in_flight,"),
    ("            dozer_core::log_info!(LOG, \"收到 Ctrl-C，退出\");\n", "            dozer_core::log_info!(LOG, \"收到 Ctrl-C，退出\");\n            apps.shutdown().await;\n"),
])

# ---- 所有构造 dozerd::server::Stores 的测试字面量 ----
patched = 0
for p in glob.glob("crates/*/tests/*.rs"):
    if p.endswith("app_requests.rs"):
        continue
    t = read(p)
    if "server::Stores {" not in t or "apps" in t:
        continue

    def sub(m):
        global patched
        patched += 1
        return m.group(0) + "\n" + m.group(1) + 'apps: dozerd::app_service::AppService::unavailable("test"),'

    t2 = re.sub(r"^(\s+)groups(?::[^\n]*)?,$", sub, t, flags=re.M)
    if t2 != t:
        write(p, t2)
print("Stores 字面量已补 apps:", patched, "处")

# ---- dozer-client ----
edit("crates/dozer-client/Cargo.toml", [("dozer-core = { path = \"../dozer-core\" }\n", "dozer-core = { path = \"../dozer-core\" }\n# 应用宿主的线上类型(默认 feature,只有 serde)\nbytehost-apps = { path = \"../bytehost-apps\" }\n")])
s = read("crates/dozer-client/src/lib.rs")
a = "use base64::engine::general_purpose::STANDARD as B64;\n"
assert a in s
s = s.replace(a, a + "use bytehost_apps::id::AppId;\nuse bytehost_apps::plan::{ApprovedInstallPlan, InstallPlan, Provenance, TrustLevel};\nuse bytehost_apps::proto::{AppReply, AppRequest, AppSource, AppSummary, RuntimeProbe};\nuse bytehost_apps::registry::UninstallMode;\n", 1)
a = "    /// 触发某 cwd 下缺失总结会话的批量补录(项目\"修复\"按钮用,spec"
assert a in s
s = s.replace(a, """    /// 应用宿主请求(原样转给 dozerd 的 `AppService`)。失败(含"应用宿主不可用")走 `Err`。
    pub async fn app_request(&self, request: AppRequest) -> Result<AppReply> {
        match self.roundtrip(&Request::App { request }).await? {
            Reply::App { reply } => Ok(reply),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn app_list(&self) -> Result<Vec<AppSummary>> {
        match self.app_request(AppRequest::List).await? {
            AppReply::Apps { apps } => Ok(apps),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 出安装计划(不安装任何东西)。
    pub async fn app_plan(
        &self,
        source: AppSource,
        provenance: Provenance,
        trust: TrustLevel,
    ) -> Result<InstallPlan> {
        match self
            .app_request(AppRequest::Plan { source, provenance, trust })
            .await?
        {
            AppReply::Plan { plan } => Ok(*plan),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 安装一份已批准的计划(服务端对 staging 副本重新计算并核对)。
    pub async fn app_install(&self, approved: ApprovedInstallPlan, source: AppSource) -> Result<()> {
        let request = AppRequest::Install { approved: Box::new(approved), source };
        self.app_expect_done(request).await
    }

    /// 启动应用,返回不含令牌的站点地址。
    pub async fn app_start(&self, id: AppId) -> Result<String> {
        match self.app_request(AppRequest::Start { id }).await? {
            AppReply::Started { url } => Ok(url),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn app_stop(&self, id: AppId) -> Result<()> {
        self.app_expect_done(AppRequest::Stop { id }).await
    }

    pub async fn app_uninstall(&self, id: AppId, mode: UninstallMode) -> Result<()> {
        self.app_expect_done(AppRequest::Uninstall { id, mode }).await
    }

    /// 首次导航用的地址(含令牌,**秘密**:不要写日志)。应用没在运行会失败。
    pub async fn app_launch_url(&self, id: AppId) -> Result<String> {
        match self.app_request(AppRequest::LaunchUrl { id }).await? {
            AppReply::LaunchUrl { url } => Ok(url),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn app_probe_runtimes(&self) -> Result<Vec<RuntimeProbe>> {
        match self.app_request(AppRequest::ProbeRuntimes).await? {
            AppReply::Runtimes { runtimes } => Ok(runtimes),
            other => bail!("意外应答: {other:?}"),
        }
    }

    async fn app_expect_done(&self, request: AppRequest) -> Result<()> {
        match self.app_request(request).await? {
            AppReply::Done => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

""" + a, 1)
write("crates/dozer-client/src/lib.rs", s)

# ---- 依赖门禁:再检查 dozer-hook 的依赖闭包 ----
s = read("scripts/check-bytehost-apps-deps.sh")
s = s.replace("tree() {\n  local out\n  out=$(cargo tree -p bytehost-apps -e normal,build --target all --prefix none \"$@\" 2>&1) || {", "tree_of() {\n  local pkg=\"$1\"; shift\n  local out\n  out=$(cargo tree -p \"$pkg\" -e normal,build --target all --prefix none \"$@\" 2>&1) || {", 1)
assert "tree_of()" in s
s = s.replace("all=$(tree --all-features)", "tree() { tree_of bytehost-apps \"$@\"; }\nall=$(tree --all-features)", 1)
a = 'echo "bytehost-apps deps check: ok"'
assert a in s
s = s.replace(a, """# 3. `dozer-core` 依赖 bytehost-apps(默认 feature),`dozer-hook` 又依赖 `dozer-core`:hook 的依赖闭包里除了
#    bytehost-apps 本身不能出现 tokio/hyper/sha2/toml/uuid 这些 server/digest/manifest-toml feature 的依赖。
hook=$(tree_of dozer-hook)
if echo "$hook" | grep -qE '^(tokio|hyper|hyper-util|http-body-util|bytes|sha2|toml|uuid)$'; then
  echo "dozer-hook 的依赖闭包被 bytehost-apps 的可选依赖污染了:" >&2
  echo "$hook" | grep -E '^(tokio|hyper|hyper-util|http-body-util|bytes|sha2|toml|uuid)$' >&2
  exit 1
fi
""" + a, 1)
write("scripts/check-bytehost-apps-deps.sh", s)
print("ok")
```

- [ ] **Step 5: 应用实现并跑测试**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a2 && export PYTHONDONTWRITEBYTECODE=1
SP=$SCRATCH/a2 python3 $SCRATCH/a2_impl.py && cargo fmt --all && cargo metadata --format-version 1 >/dev/null && git status --short | head -40
for spec in "-p bytehost-apps" "-p bytehost-apps --all-features" "-p dozer-core" "-p dozerd --lib" "-p dozerd --test app_requests" "-p dozer-client" "-p dozer-mcp -p dozer-hook"; do printf "[%s] " "$spec"; cargo test $=spec 2>&1 | grep -E "^error|FAILED|test result: .* [1-9][0-9]* passed" | awk '{print $4,$5}' | tr '\n' ' '; echo; done
for i in 1 2 3; do cargo test -p dozerd --lib app_service 2>&1 | grep -E "FAILED|test result"; done
```
Expected: 脚本输出 `Stores 字面量已补 apps: 24 处` 与 `ok`;`bytehost-apps` **59 / 132**,`dozer-core` **98**,`dozerd --lib` **471**,`app_requests` **3**,`dozer-client`/`dozer-mcp`/`dozer-hook` 与基线一致;`app_service` 测试连跑 3 遍都是 8 passed、没有失败。若某个 `assert` 报 `AssertionError`,说明 `main` 上这些位置在 A1 之后被改动——按报错点手工对齐,**不要**改断言。

- [ ] **Step 6: 全 workspace、clippy、门禁、锁文件**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a2
cargo check --workspace --all-targets 2>&1 | grep -E "^error|Finished" | head -3
cargo test -p dozer-app 2>&1 | grep -E "^error|test result|FAILED"; cat $SCRATCH/dozer-app-baseline.txt | tail -2
cargo clippy -p bytehost-apps -p dozer-core -p dozer-client -p dozerd --all-targets --all-features 2>&1 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c
./scripts/check-bytehost-apps-deps.sh; ./scripts/check-log-scope.sh
cargo tree -p dozer-hook -e normal --prefix none | awk '{print $1}' | sort -u | tr '\n' ' '; echo
git diff Cargo.lock | grep '^[-+]' | grep -v '^+++\|^---'
```
Expected: `cargo check` 以 `Finished` 结束;`dozer-app` 与基线完全相同(`1776 passed; 1 failed`);clippy 只剩 `crates/dozerd/src/preview_commands.rs` 一条(既有);两个门禁 `ok`;`dozer-hook` 闭包里只多了 `bytehost-apps`(`anyhow bytehost-apps directories dirs-sys dozer-core dozer-hook itoa libc memchr option-ext proc-macro2 quote serde serde_core serde_derive serde_json syn unicode-ident zmij`);`git diff Cargo.lock` 里只有 `+ "bytehost-apps",` 这样的依赖边(`dozer-client`、`dozer-core`、`dozerd` 三块各一行),**没有 `-` 行、没有新的 `[[package]]`**。

- [ ] **Step 7: 变异检验(先暂存)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a2 && export PYTHONDONTWRITEBYTECODE=1
git add -A crates scripts Cargo.lock
cat > $SCRATCH/a2_mutate.py <<'EOF'
import sys
w = sys.argv[1]
S = "crates/"
M = {
 "1": (S+"dozerd/src/server.rs", "                                apps.shutdown().await;\n", ""),
 "2": (S+"dozerd/src/app_service.rs", "let failures = tokio::task::spawn_blocking(move || for_reconcile.reconcile())", "let failures = tokio::task::spawn_blocking(move || { let _ = &for_reconcile; Vec::<(bytehost_apps::id::AppId, bytehost_apps::manager::ManagerError)>::new() })"),
 "3": (S+"dozerd/src/app_service.rs", "Some(a) if a.url.is_some() => Ok(m.launch_url(&id)),", "Some(_) => Ok(m.launch_url(&id)),"),
 "4": (S+"bytehost-apps/src/manager.rs", "            self.gateway.remove_site(&record.id);\n            if matches!(record.observed, ObservedState::Running)", "            if matches!(record.observed, ObservedState::Running)"),
 "5": (S+"bytehost-apps/src/manager.rs", "            self.gateway.remove_site(&record.id);\n            if matches!(record.observed, ObservedState::Running)", "            self.gateway.remove_site(&record.id);\n            record.desired = DesiredState::Stopped;\n            if matches!(record.observed, ObservedState::Running)"),
 "6": (S+"dozerd/src/server.rs", "                        Request::App { request } => match apps.handle(request).await {\n                            Ok(reply) => Reply::App { reply },\n                            Err(message) => Reply::Error { message },\n                        },", "                        Request::App { request } => match apps.handle(request).await {\n                            Ok(reply) => Reply::App { reply },\n                            Err(_message) => Reply::Ok,\n                        },"),
 "7": (S+"bytehost-apps/src/gateway/mod.rs", "        let task = self.task.lock().expect(\"task 锁\").take();", "        let task: Option<tokio::task::JoinHandle<()>> = None;"),
 "8": (S+"dozerd/src/app_service.rs", "            Err(e) => {\n                dozer_core::log_error!(LOG, error = %e, \"读取 gateway 端口失败,应用宿主不可用\");\n                Self::unavailable(format!(\"应用宿主不可用:{e}\"))\n            }", "            Err(_e) => Self::unavailable(String::new()),"),
}
p, a, b = M[w]
s = open(p).read()
assert a in s, (w, a[:60])
open(p, "w").write(s.replace(a, b, 1))
EOF
for m in 1 2 3 4 5 6 7 8; do printf "M$m: "; python3 $SCRATCH/a2_mutate.py $m 2>&1 | tail -1; r=$(cargo test -p dozerd -p bytehost-apps --all-features --test app_requests --lib 2>&1 | grep -E "FAILED$|^error(\[|:)" | head -2 | cut -c1-130 | tr '\n' '|'); echo "${r:-NOT CAUGHT}"; git checkout -- crates; done
git diff --stat | wc -l
# 门禁变异:让 dozer-core 打开 manifest-toml,hook 的闭包就会被 toml 污染
cp crates/dozer-core/Cargo.toml $SCRATCH/core.toml.bak
sed -i '' 's|^bytehost-apps = { path = "../bytehost-apps" }|bytehost-apps = { path = "../bytehost-apps", features = ["manifest-toml"] }|' crates/dozer-core/Cargo.toml
./scripts/check-bytehost-apps-deps.sh 2>&1 | head -3; echo "exit=${pipestatus[1]}"
cp $SCRATCH/core.toml.bak crates/dozer-core/Cargo.toml; ./scripts/check-bytehost-apps-deps.sh; git diff --stat | wc -l
```
Expected: 八次变异**每一次**都以 `FAILED` 收场(没有 `NOT CAUGHT`):M1(`Shutdown` 不调 `apps.shutdown()`):`a_shutdown_request_takes_the_apps_down_but_keeps_what_the_user_wanted`;M2(启动不对账):`apps_wanted_running_come_back_after_a_dozerd_restart…`;M3(`LaunchUrl` 对没在运行的应用也发地址):`launch_urls_are_only_given_for_running_apps`;M4(`suspend_all` 不撤站点)与 M5(`suspend_all` 把 `desired` 改成 `Stopped`):`suspending_keeps_what_the_user_wanted_so_the_next_start_restores_it`;M6(错误被吞成 `Ok`):`an_unavailable_app_host_does_not_break_the_rest_of_the_daemon` 与 `the_whole_install_run_stop_uninstall_cycle_works_over_the_socket`;M7(`stop` 不等 accept 循环):`stop_is_idempotent_and_marks_the_gateway_stopped`;M8(端口文件读失败时不报原因):`a_corrupt_gateway_port_file_makes_the_service_unavailable_and_is_left_alone`;每次还原后 `git diff --stat | wc -l` 为 `0`。门禁变异:输出 `dozer-hook 的依赖闭包被 bytehost-apps 的可选依赖污染了:` 与 `toml`,`exit=1`;还原后门禁 `ok`。

- [ ] **Step 8: 提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a2
git status --short | head -40
git add crates scripts Cargo.lock
git diff --cached --stat | tail -12
git commit -m "feat(dozerd): app host — AppService, Request::App/Reply::App, startup reconcile, shutdown that keeps desired state

Wire the bytehost app host into dozerd (apps follow dozerd). The wire types AppRequest/AppReply live in
bytehost-apps::proto (serde only) and dozer-core::protocol carries them as Request::App/Reply::App.
AppService never fails startup (a taken port or a broken app dir only makes it unavailable, with the reason
in every error), reconciles on start, and on Shutdown/Ctrl-C takes sites down while keeping desired=Running
so the next start restores them. Every AppManager call goes through spawn_blocking. dozer-client gains
app_* methods. The deps gate now also checks that dozer-hook's dependency closure stays free of the
server/digest/manifest-toml dependencies.

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: `cargo check` 之后的暂存区包含 `crates/{bytehost-apps,dozer-core,dozer-client,dozerd,dozer-mcp}/**`(含 24 处测试字面量)、`scripts/check-bytehost-apps-deps.sh`、`Cargo.lock`;**没有** `docs/`、`CLAUDE.md`、`.cargo/`。

---

### Task 2: 文档回填

**Files:**
- Modify: `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`、`docs/superpowers/specs/2026-10-04-bytehost-extraction-roadmap.md`、`CLAUDE.md`

**Interfaces:**
- Consumes: Task 1 的提交。

- [ ] **Step 1: 回填并校验**

用带引号的 heredoc 创建 `$SCRATCH/a2_docs.py`(提交短 id 经环境变量传入):

```python
import os
A2 = os.environ["A2"]
sp = "docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md"
lines = open(sp).read().split("\n")
hit = False
for k, l in enumerate(lines):
    if l.startswith("| **A2** |"):
        lines[k] = l[:-1].rstrip() + " **已完成(A2,`bytehost-a2`):`" + A2 + "`**——线上类型在 `bytehost-apps::proto`;`dozer-core::protocol` 加 `Request::App`/`Reply::App`;dozerd 的 `AppService`(启动永不失败、启动对账、`Shutdown` 时收尾且保留 `desired`);`dozer-client` 的 `app_*` 方法;依赖门禁新增 `dozer-hook` 闭包检查。**推送事件**(GUI 订阅状态变化)本切片没做,A4 之前 GUI 靠轮询 `List`;启动时的\"首次绑定成功后才持久化端口\"也留给 A4 前的小修 |"
        hit = True
assert hit
open(sp, "w").write("\n".join(lines))
rp = "docs/superpowers/specs/2026-10-04-bytehost-extraction-roadmap.md"
lines = open(rp).read().split("\n")
hit = False
for k, l in enumerate(lines):
    if l.startswith("- **A2** "):
        lines[k] = l + " **已完成(`bytehost-a2`):`" + A2 + "`。**"
        hit = True
assert hit
open(rp, "w").write("\n".join(lines))
cp = "CLAUDE.md"
s = open(cp).read()
a = "只有 dozerd 打开 |\n"
assert a in s
s = s.replace(a, "只有 dozerd 打开;GUI↔dozerd 的线上类型 `AppRequest`/`AppReply` 在其 `proto` 模块,dozerd 里由 `app_service.rs` 承接(`Request::App`/`Reply::App`)。**应用跟随 dozerd**:dozerd 退出时撤下站点但保留 `desired`,下次启动自动恢复 |\n", 1)
open(cp, "w").write(s)
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a2 && export PYTHONDONTWRITEBYTECODE=1
A2=$(git log --format=%h -1 --grep="app host — AppService") python3 $SCRATCH/a2_docs.py
git diff -- docs CLAUDE.md | grep -E "^[-+]" | grep -vE "^(\+\+\+|---)" | cut -c1-170
./scripts/check-bytehost-apps-deps.sh
```
Expected: `git diff` 里只有:应用宿主规格里 A2 行的"已完成"标记、路线图 A2 行的"已完成"标记、`CLAUDE.md` 里 `crates/bytehost-apps` 一行末尾多出关于线上类型与"应用跟随 dozerd"的说明;**反引号内容完整**;门禁 `ok`。

- [ ] **Step 2: 提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a2
git status --short
git add docs CLAUDE.md
git diff --cached --stat | tail -5
git commit -m "docs(bytehost): backfill A2 (app host wired into dozerd)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: 只有 `docs/` 下 2 个文件与 `CLAUDE.md`。

---

## Self-Review

**1. 覆盖:** 规格 §7 的 A2 项——`dozer-core::protocol` 加 `Request::App`/`Reply::App` ✔、`dozerd/server.rs` 转给 `AppManager` ✔(经 `AppService`)、`dozer-client` 的 `app_*` ✔、dozerd 启动对账 ✔、优雅退出停应用 ✔(保留 `desired`)、孤儿清理(静态应用没有进程,`reconcile` 的 `recover_after_supervisor_restart` 已覆盖"崩溃后观察态陈旧")✔。规格 §6.1 的"gateway 放进 dozerd"✔。**未覆盖并在 Review Focus 说明:** 推送事件、端口首次绑定后才持久化、SIGTERM 收尾(靠对账)、GUI。

**2. 占位符扫描:** 测试、实现、脚本都是草稿里跑通的完整版本;**草稿把脚本在全新 worktree 上重放,与原型树逐字节比对(`git diff --cached` 完全一致)**;随后的全量测试数、clippy、门禁、8+1 个变异均如上所述。

**3. 一致性:** `AppRequest`/`AppReply`/`AppService`/`Stores.apps`/`Client::app_*` 的名字与形状在测试、实现、Interfaces、文档里一致。

**4. Review Focus:** 9 条各有归属(1→M1 与 A1 的对账测试;2→corrupt/taken-port 测试;3→说明;4→说明与审阅;5→说明;6→脚本 + `cargo check --workspace --all-targets`;7→范围说明;8→说明;9→范围说明)。

---

## 执行后修订(评审修复轮,2026-10-05)

独立评审(opus)对执行出的分支提出 2 条 Important:

1. **成立并已修复——`Request::App` 在 dozerd 停止期间/之后仍被接受。** 一个恰好排在 manager 锁后面的 `Start` 会在 `suspend_all` 撤掉站点之后拿到锁、把站点注册回已停止的 gateway,并回一个永远打不开的 `Started{url}`(将来进程型 runtime 还会因此留下没人管的子进程)。现在 `AppManager` 有 `closed` 标志:`suspend_all` 置位、`reconcile` 清除;置位期间 `install` 与 `start` 在**拿到锁之后**返回新的 `ManagerError::ShuttingDown`(停止/卸载这类清理操作照常允许)。两个新测试:`after_suspend_all_installs_and_starts_are_refused_until_the_next_reconcile`(manager)与 `requests_arriving_after_shutdown_do_not_start_or_install_anything`(`AppService`);3 个变异(去掉 `start`/`install` 的检查、`reconcile` 不重新放开)都被抓到。测试数:`bytehost-apps --all-features` 132 → **133**,`dozerd --lib` 471 → **472**。
2. **未修复,有明确裁决——错误只是字符串。** GUI 要区分"应用宿主不可用"(持久状态)与一次性失败(Toast),目前只能匹配文字。加 `AppReply::Unavailable`/`AppError { kind, message }` 是增量改动,但它会改变所有客户端包装函数对错误的映射,属于 A4 里设计 GUI 两种呈现时一并决定的事;代价是 A4 要再动一次 `app_service.rs`/`proto.rs`/`dozer-client`。

**推迟的 Minor(9 条)**记在账本里并写进最终汇报,其中值得 A4 前处理的:关停没有时间上限且排在 manager 锁之后(安装大目录时停 dozerd 会等拷贝完)、`JoinError` 被静默吞掉且 panic 会毒化锁、`AppService` 在 `serve()` 的重复进程检查与 socket 绑定之前就启动、门禁第 3 项只查 `dozer-hook` 单独的依赖闭包(发布脚本把 hook 与 dozerd 一起构建,feature 会合并;链接器会裁掉未用代码)、`LocalDir.path` 未校验且整目录(含 `.env`)被永久拷贝。
