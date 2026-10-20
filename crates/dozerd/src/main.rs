//! dozerd 薄入口:参数解析 + 调用 dozerd::daemon::run。
//! 装配逻辑在 daemon.rs,Digger 的 diggerd 复用同一个函数、传自己的
//! 产品身份与功能开关。

use anyhow::Result;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    // 极简参数解析：仅 --socket <path>（daemon 不引 clap，保持轻）。放在
    // 任何有副作用的启动工作之前——--version 是纯查询，不该有"顺带建了
    // 个日志目录"这种副作用。
    let mut args = std::env::args().skip(1);
    let mut socket: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => match args.next() {
                Some(p) => socket = Some(PathBuf::from(p)),
                None => {
                    eprintln!("--socket 需要一个路径参数"); // cli-output
                    std::process::exit(2);
                }
            },
            "--version" => {
                println!("dozerd {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => {
                eprintln!("未知参数: {other}（支持 --socket <path> / --version）"); // cli-output
                std::process::exit(2);
            }
        }
    }
    let mut config =
        dozerd::daemon::DaemonConfig::for_product(dozer_core::paths::ProductId::dozer());
    if let Some(socket) = socket {
        config.socket = socket;
    }
    dozerd::daemon::run(config).await
}
