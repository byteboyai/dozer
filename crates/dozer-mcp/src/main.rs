use dozer_mcp::{install, server};

#[tokio::main]
async fn main() {
    let arg1 = std::env::args().nth(1);
    match arg1.as_deref() {
        Some("serve") => {
            if let Err(e) = server::run().await {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Some("install") => {
            let agent = std::env::args().nth(2).unwrap_or_else(|| "claude".into());
            std::process::exit(install::run(&agent, true));
        }
        Some("uninstall") => {
            let agent = std::env::args().nth(2).unwrap_or_else(|| "claude".into());
            std::process::exit(install::run(&agent, false));
        }
        _ => {
            eprintln!("用法: dozer-mcp <serve|install|uninstall> [agent]");
            std::process::exit(2);
        }
    }
}
