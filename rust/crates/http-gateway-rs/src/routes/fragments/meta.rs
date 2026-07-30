// Fragment of routes::app (include!). Author: kejiqing

#[utoipa::path(
    get,
    path = "/",
    tag = "System",
    operation_id = "root",
    responses(
        (status = 200, description = "Gateway welcome HTML page", content_type = "text/html")
    )
)]
pub(crate) async fn root() -> Html<&'static str> {
    Html("<h3>claw gateway rs</h3><p>Open <a href=\"/docs\">/docs</a> to view all endpoints.</p>")
}

#[utoipa::path(
    get,
    path = "/docs",
    tag = "System",
    operation_id = "docs",
    responses(
        (status = 307, description = "Redirect to Swagger UI at /docs-ui/")
    )
)]
pub(crate) async fn docs() -> axum::response::Redirect {
    axum::response::Redirect::temporary("/docs-ui/")
}


#[utoipa::path(
    get,
    path = "/openapi.json",
    tag = "System",
    operation_id = "openapi",
    responses(
        (status = 200, description = "OpenAPI 3 document derived from Rust handler/DTO types", body = Object)
    )
)]
pub(crate) async fn openapi() -> Json<Value> {
    Json(crate::openapi::document())
}
