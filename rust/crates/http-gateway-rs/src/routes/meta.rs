//! meta routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::{docs, openapi, root};
use axum::routing::get;
use axum::Router;
use utoipa_swagger_ui::{Config, SwaggerUi};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(root))
        .route("/docs", get(docs))
        .route("/openapi.json", get(openapi))
        .merge(
            SwaggerUi::new("/docs-ui").config(
                Config::new(["/openapi.json"])
                    .doc_expansion("list")
                    // Prefer Schema view so enum Allowed values are visible by default.
                    .default_model_rendering("model")
                    .default_model_expand_depth(2)
                    .display_request_duration(true)
                    .filter(true)
                    .persist_authorization(true)
                    .try_it_out_enabled(true),
            ),
        )
}
