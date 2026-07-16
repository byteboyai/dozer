use anyhow::Result;
use dozerd::registry::SessionRegistry;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    // 极简参数解析：仅 --socket <path>（daemon 不引 clap，保持轻）
    let mut args = std::env::args().skip(1);
    let mut socket: PathBuf = dozer_core::paths::socket_path();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => match args.next() {
                Some(p) => socket = PathBuf::from(p),
                None => {
                    eprintln!("--socket 需要一个路径参数");
                    std::process::exit(2);
                }
            },
            "--version" => {
                println!("dozerd {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => {
                eprintln!("未知参数: {other}（支持 --socket <path> / --version）");
                std::process::exit(2);
            }
        }
    }

    let registry = Arc::new(SessionRegistry::new());
    let serve = dozerd::server::serve(&socket, registry);
    tokio::select! {
        r = serve => r?,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("收到 Ctrl-C，退出");
            let _ = std::fs::remove_file(&socket);
        }
    }
    Ok(())
}
