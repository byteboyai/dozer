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

    /// dozerd 停止之后到达的请求不能再启动/安装任何东西:否则会给调用方一个"成功"的假应答,
    /// 并在一个已停止的 gateway 上注册站点(将来进程型 runtime 还会因此留下没人管的子进程)。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn requests_arriving_after_shutdown_do_not_start_or_install_anything() {
        let tmp = tempfile::tempdir().unwrap();
        let svc =
            AppService::start_with(&tmp.path().join("bytehost"), GatewayConfig { port: 0 }).await;
        let a = write_app(&tmp.path().join("src/a"), "alpha", "A");
        install(&svc, a).await;
        svc.shutdown().await;

        let err = svc
            .handle(AppRequest::Start { id: id("alpha") })
            .await
            .unwrap_err();
        assert!(err.contains("停止"), "{err}");
        let b = write_app(&tmp.path().join("src/b"), "beta", "B");
        let plan = svc
            .handle(AppRequest::Plan {
                source: b.clone(),
                provenance: Provenance::Local,
                trust: TrustLevel::Trusted,
            })
            .await
            .unwrap();
        let AppReply::Plan { plan } = plan else {
            panic!()
        };
        let approved = plan.approve(Approval {
            approver: "t".into(),
            approved_ms: 1,
        });
        let err = svc
            .handle(AppRequest::Install {
                approved: Box::new(approved),
                source: b,
            })
            .await
            .unwrap_err();
        assert!(err.contains("停止"), "{err}");
        let AppReply::Apps { apps } = svc.handle(AppRequest::List).await.unwrap() else {
            panic!()
        };
        assert_eq!(apps.len(), 1, "beta 没有被安装");
        assert!(apps[0].url.is_none(), "alpha 没有被再次启动");
        assert!(svc.gateway_stopped());
    }
}
