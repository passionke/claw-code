use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let bind = std::env::var("CLAW_SERVER_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let database_url =
        std::env::var("CLAW_SERVER_DATABASE_URL").unwrap_or_else(|_| "sqlite:claw-server.db".into());
    let data_dir = std::env::var("CLAW_SERVER_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./claw-server-data"));
    let master_key = std::env::var("CLAW_MASTER_KEY").map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "CLAW_MASTER_KEY must be set for API key encryption at rest",
        )
    })?;
    if master_key.len() < 16 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "CLAW_MASTER_KEY must be at least 16 characters",
        )
        .into());
    }
    let static_dir = std::env::var("CLAW_WEB_DIST").ok().map(PathBuf::from);

    claw_server::server::serve(&bind, &database_url, data_dir, master_key, static_dir).await
}
