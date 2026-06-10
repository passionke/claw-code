//! claw-sandbox binary entrypoint. Author: kejiqing

#[tokio::main]
async fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    if let Err(e) = claw_sandbox_server::run().await {
        eprintln!("claw-sandbox: {e}");
        std::process::exit(1);
    }
}
