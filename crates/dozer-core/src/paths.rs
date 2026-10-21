use directories::ProjectDirs;
use std::path::PathBuf;
use std::sync::OnceLock;

/// 产品的目录来源:标准 `ProjectDirs` 三元组,或一对固定路径
/// (Digger 这类自带数据根的产品用 `Fixed`)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProductDirs {
    Project {
        qualifier: &'static str,
        organization: &'static str,
        application: &'static str,
    },
    Fixed {
        config: PathBuf,
        state: PathBuf,
    },
}

/// 产品身份:一个产品(Dozer/Digger)守护进程全套命名——目录、
/// socket、数据库、会话环境变量、hook 二进制。同一进程只能属于
/// 一个产品(`install`),两个产品同机并存时数据互不串线。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductId {
    pub name: &'static str,
    pub session_env: &'static str,
    pub hook_bin: &'static str,
    pub socket_file: &'static str,
    pub db_file: &'static str,
    dirs: ProductDirs,
}

impl ProductId {
    /// Dozer 本尊:与产品化改造前的全部路径字节一致(黄金测试守护)。
    pub fn dozer() -> ProductId {
        ProductId {
            name: "dozer",
            session_env: "DOZER_SESSION_ID",
            hook_bin: "dozer-hook",
            socket_file: "dozerd.sock",
            db_file: "dozer.db",
            dirs: ProductDirs::Project {
                qualifier: "ai",
                organization: "byteboy",
                application: "dozer",
            },
        }
    }

    /// 构造别的产品(如 Digger):命名与目录都由调用方给定。
    pub fn custom(
        name: &'static str,
        session_env: &'static str,
        hook_bin: &'static str,
        socket_file: &'static str,
        db_file: &'static str,
        dirs: ProductDirs,
    ) -> ProductId {
        ProductId {
            name,
            session_env,
            hook_bin,
            socket_file,
            db_file,
            dirs,
        }
    }

    /// ~/Library/Application Support 或 XDG config 下的产品配置目录
    pub fn config_dir(&self) -> PathBuf {
        match &self.dirs {
            ProductDirs::Project {
                qualifier,
                organization,
                application,
            } => dirs_from(qualifier, organization, application)
                .config_dir()
                .to_path_buf(),
            ProductDirs::Fixed { config, .. } => config.clone(),
        }
    }

    /// 运行态（PID、socket、滚屏缓存）目录
    pub fn state_dir(&self) -> PathBuf {
        match &self.dirs {
            ProductDirs::Project {
                qualifier,
                organization,
                application,
            } => {
                let dirs = dirs_from(qualifier, organization, application);
                dirs.state_dir()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| dirs.data_local_dir().to_path_buf())
            }
            ProductDirs::Fixed { state, .. } => state.clone(),
        }
    }

    /// 该产品守护进程的 Unix Domain Socket 路径
    pub fn socket_path(&self) -> PathBuf {
        self.state_dir().join(self.socket_file)
    }

    /// 滚动日志文件目录(守护进程按天切分的 tracing 输出)。
    pub fn logs_dir(&self) -> PathBuf {
        self.state_dir().join("logs")
    }

    /// 该产品守护进程的数据库文件路径
    pub fn db_path(&self) -> PathBuf {
        self.state_dir().join(self.db_file)
    }

    /// 该产品的 MCP 二进制名(`dozer-mcp` / `digger-mcp`)。
    pub fn mcp_bin(&self) -> String {
        format!("{}-mcp", self.name)
    }

    /// 该产品用于配置/环境变量覆盖的前缀(`DOZER` / `DIGGER`)。
    pub fn env_prefix(&self) -> String {
        self.name.to_uppercase()
    }
}

/// 已知产品的会话环境变量名(只含名字,不带值)。安静模式据此判断
/// "别的产品的会话变量在、自己的不在"。新增第三个产品时须同步这里;
/// 测试会核对两个产品的 `session_env` 都在列表中。
pub const KNOWN_SESSION_ENVS: &[&str] = &["DOZER_SESSION_ID", "DIGGER_SESSION_ID"];

/// 是否存在**别的**已知产品的会话变量,且值非空。`own` 是当前产品的
/// 会话变量名(不参与判断)。
pub fn other_known_session_env_present(own: &str, env: &dyn Fn(&str) -> Option<String>) -> bool {
    KNOWN_SESSION_ENVS
        .iter()
        .filter(|name| **name != own)
        .any(|name| env(name).is_some_and(|v| !v.is_empty()))
}

fn dirs_from(
    qualifier: &'static str,
    organization: &'static str,
    application: &'static str,
) -> ProjectDirs {
    ProjectDirs::from(qualifier, organization, application).expect("home directory must exist")
}

/// 进程级产品身份:第一个调用者赢。守护进程/hook 入口启动时先
/// `install`,之后的自由函数(`config_dir()` 等)都读它。
static PRODUCT: OnceLock<ProductId> = OnceLock::new();

/// 安装进程级产品身份。只能成功一次:重复安装**同值**返回 Ok
/// (幂等),安装**不同值**返回 Err 并带回已装的那个——同一进程
/// 绝不能同时是两个产品。
pub fn install(product: ProductId) -> Result<(), Box<InstallProductError>> {
    if PRODUCT.set(product.clone()).is_ok() {
        return Ok(());
    }
    let installed = PRODUCT.get().expect("set 失败意味着已有值");
    if installed == &product {
        Ok(())
    } else {
        Err(Box::new(InstallProductError {
            installed: installed.clone(),
            attempted: product,
        }))
    }
}

/// 当前进程的产品身份;未调用 `install` 时固定为 Dozer。
pub fn product() -> &'static ProductId {
    PRODUCT.get_or_init(ProductId::dozer)
}

/// `install` 拒绝第二个产品时返回的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallProductError {
    /// 进程里已经装上的产品
    pub installed: ProductId,
    /// 被拒绝的产品
    pub attempted: ProductId,
}

impl std::fmt::Display for InstallProductError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "进程已安装产品 \"{}\",拒绝再安装 \"{}\"(一个进程只能是同一家产品)",
            self.installed.name, self.attempted.name
        )
    }
}

impl std::error::Error for InstallProductError {}

/// 进程当前产品的配置目录(默认 Dozer;由 `install` 固定)。
pub fn config_dir() -> PathBuf {
    product().config_dir()
}

/// 进程当前产品的运行态目录。
pub fn state_dir() -> PathBuf {
    product().state_dir()
}

/// 进程当前产品守护进程的 socket 路径。
pub fn socket_path() -> PathBuf {
    product().socket_path()
}

/// 进程当前产品的滚动日志目录。
pub fn logs_dir() -> PathBuf {
    product().logs_dir()
}

/// 进程当前产品的会话环境变量名(如 `DOZER_SESSION_ID`)。
/// 除 `ProductId::dozer()` 的定义与测试外,代码里不得硬编码变量名
/// (守护测试 no_session_env_literal 守护这一点)。
pub fn session_env() -> &'static str {
    product().session_env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digger_at(root: &std::path::Path) -> ProductId {
        ProductId::custom(
            "digger",
            "DIGGER_SESSION_ID",
            "digger-hook",
            "diggerd.sock",
            "digger.db",
            ProductDirs::Fixed {
                config: root.join("config"),
                state: root.join("state"),
            },
        )
    }

    #[test]
    fn config_dir_ends_with_dozer() {
        assert!(config_dir().to_string_lossy().contains("dozer"));
    }

    #[test]
    fn state_dir_ends_with_dozer() {
        assert!(state_dir().to_string_lossy().contains("dozer"));
    }

    #[test]
    fn socket_path_is_under_state_dir() {
        let s = socket_path();
        assert!(s.starts_with(state_dir()));
        assert_eq!(s.file_name().unwrap(), "dozerd.sock");
    }

    #[test]
    fn logs_dir_is_under_state_dir() {
        let d = logs_dir();
        assert!(d.starts_with(state_dir()));
        assert_eq!(d.file_name().unwrap(), "logs");
    }

    #[test]
    fn custom_product_derives_its_own_paths() {
        let root = std::path::Path::new("/data/digger");
        let digger = digger_at(root);
        assert_eq!(digger.config_dir(), root.join("config"));
        assert_eq!(digger.state_dir(), root.join("state"));
        assert_eq!(
            digger.socket_path(),
            root.join("state").join("diggerd.sock")
        );
        assert_eq!(digger.db_path(), root.join("state").join("digger.db"));
        assert_eq!(digger.logs_dir(), root.join("state").join("logs"));
        assert_eq!(digger.session_env, "DIGGER_SESSION_ID");
        assert_eq!(digger.hook_bin, "digger-hook");
    }

    #[test]
    fn two_products_never_share_paths() {
        let dozer = ProductId::dozer();
        let digger = digger_at(tempfile::tempdir().unwrap().path());
        let dozer_paths = [
            dozer.config_dir(),
            dozer.state_dir(),
            dozer.socket_path(),
            dozer.logs_dir(),
        ];
        let digger_paths = [
            digger.config_dir(),
            digger.state_dir(),
            digger.socket_path(),
            digger.logs_dir(),
        ];
        for a in &dozer_paths {
            for b in &digger_paths {
                assert!(
                    !a.starts_with(b) && !b.starts_with(a),
                    "{a:?} 与 {b:?} 不应互相包含"
                );
            }
        }
    }
    #[test]
    fn mcp_bin_and_env_prefix_are_derived_from_the_name() {
        assert_eq!(ProductId::dozer().mcp_bin(), "dozer-mcp");
        assert_eq!(ProductId::dozer().env_prefix(), "DOZER");
        let tmp = std::env::temp_dir();
        let digger = digger_at(&tmp);
        assert_eq!(digger.mcp_bin(), "digger-mcp");
        assert_eq!(digger.env_prefix(), "DIGGER");
    }

    #[test]
    fn other_known_session_env_present_ignores_the_own_variable_and_empty_values() {
        let env = |name: &str| -> Option<String> {
            match name {
                "DOZER_SESSION_ID" => Some("dozer-session".into()),
                "DIGGER_SESSION_ID" => Some("digger-session".into()),
                _ => None,
            }
        };
        assert!(other_known_session_env_present("DOZER_SESSION_ID", &env));
        assert!(other_known_session_env_present("DIGGER_SESSION_ID", &env));

        let empty =
            |name: &str| -> Option<String> { (name == "DIGGER_SESSION_ID").then(String::new) };
        assert!(!other_known_session_env_present("DOZER_SESSION_ID", &empty));

        let none = |_name: &str| -> Option<String> { None };
        assert!(!other_known_session_env_present("DOZER_SESSION_ID", &none));
    }

    #[test]
    fn known_session_envs_cover_both_products_default_session_env_names() {
        let tmp = std::env::temp_dir();
        let digger = digger_at(&tmp);
        assert!(KNOWN_SESSION_ENVS.contains(&ProductId::dozer().session_env));
        assert!(KNOWN_SESSION_ENVS.contains(&digger.session_env));
        // 名单里不应有重复。
        let unique: std::collections::HashSet<_> = KNOWN_SESSION_ENVS.iter().collect();
        assert_eq!(unique.len(), KNOWN_SESSION_ENVS.len());
    }
}
