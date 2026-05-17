//! ag-ui-claw-bridge binary. Author: kejiqing

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let addr = std::env::var("CLAW_AGUI_BRIDGE_ADDR").unwrap_or_else(|_| "0.0.0.0:8090".into());
    if let Err(e) = ag_ui_claw_bridge::serve(&addr).await {
        eprintln!("ag-ui-claw-bridge: {e}");
        std::process::exit(1);
    }
}
