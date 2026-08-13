//! 只读探针：连接正在跑的 dozerd，打印每个项目当前的 PreviewContext。
//! 临时验证用，不改任何状态。
use dozer_client::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::new(dozer_core::paths::socket_path());
    let projects = client.list_projects().await?;
    if projects.is_empty() {
        println!("(no projects)");
        return Ok(());
    }
    for p in projects {
        let ctx = client.get_preview_context(p.id).await?;
        println!("project id={} name={} -> {:#?}", p.id, p.name, ctx);
    }
    Ok(())
}
