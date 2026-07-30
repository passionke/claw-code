//! project_config routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::{
    activate_project_config_version, commit_project_config_draft, compare_project_config_versions,
    compare_project_entity_versions, delete_project_config_version, get_project_config,
    list_project_config_versions, list_project_entity_versions, patch_project_config_version_note,
    put_project_config, restore_project_entity_revision,
};
use axum::routing::{delete, get, post};
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/project/config/{proj_id}",
            get(get_project_config).put(put_project_config),
        )
        .route(
            "/v1/project/config/{proj_id}/versions",
            get(list_project_config_versions),
        )
        .route(
            "/v1/project/config/{proj_id}/versions/compare",
            get(compare_project_config_versions),
        )
        .route(
            "/v1/project/config/{proj_id}/entities/{domain}/{entity_key}/versions/compare",
            get(compare_project_entity_versions),
        )
        .route(
            "/v1/project/config/{proj_id}/entities/{domain}/{entity_key}/versions",
            get(list_project_entity_versions),
        )
        .route(
            "/v1/project/config/{proj_id}/entities/{domain}/{entity_key}/restore",
            post(restore_project_entity_revision),
        )
        .route(
            "/v1/project/config/{proj_id}/versions/commit",
            post(commit_project_config_draft),
        )
        .route(
            "/v1/project/config/{proj_id}/versions/{content_rev}",
            delete(delete_project_config_version).patch(patch_project_config_version_note),
        )
        .route(
            "/v1/project/config/{proj_id}/versions/{content_rev}/activate",
            post(activate_project_config_version),
        )
}
