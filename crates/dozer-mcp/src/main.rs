use dozer_core::paths::ProductId;
use dozer_mcp::McpFeatures;

fn main() {
    dozer_mcp::server::run_main(ProductId::dozer(), McpFeatures::all());
}
