use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    let state = dozer_core::paths::state_dir();
    std::fs::create_dir_all(&state)?;
    tracing::info!(state_dir = %state.display(), "dozerd 骨架启动（P1b 实现会话内核）");
    Ok(())
}
