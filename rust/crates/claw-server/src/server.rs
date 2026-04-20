use std::path::PathBuf;

use axum::http::Method;
use axum::routing::{delete, get, post};
use axum::Router;
use sqlx::SqlitePool;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

use crate::auth;
use crate::chat;
use crate::db;
use crate::profiles;
use crate::workspaces;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub data_dir: PathBuf,
    pub master_key: String,
}

pub async fn serve(
    bind: &str,
    database_url: &str,
    data_dir: PathBuf,
    master_key: String,
    static_dir: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all(&data_dir)?;
    let pool = db::connect(database_url).await?;
    let state = AppState {
        pool,
        data_dir: data_dir.clone(),
        master_key,
    };

    let api = Router::new()
        .route("/api/register", post(auth::register))
        .route("/api/login", post(auth::login))
        .route("/api/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        .route("/api/workspaces", get(workspaces::list_workspaces))
        .route("/api/workspaces", post(workspaces::create_workspace))
        .route("/api/workspaces/:id", delete(workspaces::delete_workspace))
        .route("/api/provider-profiles", get(profiles::list_profiles))
        .route("/api/provider-profiles", post(profiles::create_profile))
        .route(
            "/api/provider-profiles/:id",
            delete(profiles::delete_profile),
        )
        .route("/api/chat", post(chat::chat_json))
        .route("/api/chat/stream", get(chat::chat_sse))
        .with_state(state.clone());

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers(Any);

    let mut app = Router::new().merge(api).layer(cors);

    if let Some(dir) = static_dir {
        if dir.is_dir() {
            app = app.fallback_service(ServeDir::new(dir));
        }
    }

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!("listening on http://{bind}");
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}
