use dozer_client::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::new(dozer_core::paths::socket_path());
    let sessions = client.list().await?;
    println!("{} session(s):", sessions.len());
    for s in sessions {
        println!("  id={} project_id={:?} alive={:?}", s.id, s.project_id, s);
    }
    Ok(())
}
