//! HTTP routes. Author: kejiqing
pub(crate) mod admin_mcp;
pub(crate) mod app;
pub(crate) mod biz_report;
pub(crate) mod gateway_settings;
pub(crate) mod health;
pub(crate) mod delegate;
pub(crate) mod master;
pub(crate) mod mcp;
pub(crate) mod meta;
pub(crate) mod pools;
pub(crate) mod project_assets;
pub(crate) mod project_config;
pub(crate) mod project_inference;
pub(crate) mod projects;
pub(crate) mod sessions;
pub(crate) mod solve;
pub(crate) mod tasks;
pub(crate) mod turns;

use axum::middleware;
use axum::Router;
use tower_http::trace::TraceLayer;
use tracing::field::Empty;

use crate::app_state::{AppState, HttpRequestId};
use crate::routes::app::inject_http_request_id;

pub(crate) fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(health::router())
        .merge(meta::router())
        .merge(projects::router())
        .merge(project_inference::router())
        .merge(solve::router())
        .merge(tasks::router())
        .merge(sessions::router())
        .merge(pools::router())
        .merge(turns::router())
        .merge(biz_report::router())
        .merge(project_assets::router())
        .merge(project_config::router())
        .merge(gateway_settings::router())
        .merge(admin_mcp::router())
        .merge(master::router())
        .merge(delegate::router())
        .merge(mcp::router())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &http::Request<axum::body::Body>| {
                    let request_id = request
                        .extensions()
                        .get::<HttpRequestId>()
                        .map_or("-", |h| h.0.as_str());
                    tracing::info_span!(
                        "http_request",
                        http.method = %request.method(),
                        http.uri = %request.uri(),
                        http.version = ?request.version(),
                        request_id = %request_id,
                        http.status_code = Empty,
                        latency_ms = Empty,
                    )
                })
                .on_response(
                    |response: &http::Response<axum::body::Body>,
                     latency: std::time::Duration,
                     span: &tracing::Span| {
                        span.record(
                            "http.status_code",
                            tracing::field::display(response.status().as_u16()),
                        );
                        span.record("latency_ms", latency.as_millis() as u64);
                    },
                ),
        )
        .layer(middleware::from_fn(inject_http_request_id))
        .with_state(state)
}
