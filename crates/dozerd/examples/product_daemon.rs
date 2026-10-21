//! S6a Task 5 进程级隔离测试的辅助守护进程:以 --name/--root 参数构造
//! 临时产品身份 + DaemonFeatures::minimal() 后调 dozerd::daemon::run。
//! 两个 OS 进程各装一个产品,验证数据/事件互不串线。

use dozer_core::paths::{ProductDirs, ProductId};
use dozerd::daemon::{DaemonConfig, DaemonFeatures};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut name = String::new();
    let mut root = std::path::PathBuf::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--name" => name = args.next().expect("--name <n>"),
            "--root" => root = std::path::PathBuf::from(args.next().expect("--root <dir>")),
            other => anyhow::bail!("未知参数: {other}(支持 --name <n> --root <dir>)"),
        }
    }
    anyhow::ensure!(
        !name.is_empty() && !root.as_os_str().is_empty(),
        "需要 --name 与 --root"
    );
    let product = ProductId::custom(
        // 名字只在测试进程存活期内使用,leak 成 'static
        Box::leak(name.into_boxed_str()),
        "TEST_SESSION_ID",
        "test-hook",
        "testd.sock",
        "test.db",
        ProductDirs::Fixed {
            config: root.join("config"),
            state: root.join("state"),
        },
    );
    let socket = product.socket_path();
    let config = DaemonConfig {
        product,
        socket,
        features: DaemonFeatures::minimal(),
    };
    dozerd::daemon::run(config).await
}
