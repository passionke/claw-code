//! project_assets routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::{
    get_effective_prompt, get_proj_skill, get_project_claude_md, get_project_tools_catalog,
    list_proj_skills, post_effective_prompt, update_project_claude_md, upsert_project_skill,
};
use axum::routing::{get, post};
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/project/claude/{proj_id}",
            get(get_project_claude_md).post(update_project_claude_md),
        )
        .route("/v1/project/skills/{proj_id}", post(upsert_project_skill))
        .route(
            "/v1/project/prompt/{proj_id}/effective",
            get(get_effective_prompt).post(post_effective_prompt),
        )
        .route("/v1/project/tools/catalog", get(get_project_tools_catalog))
        .route("/v1/skills/{proj_id}/{skill_name}", get(get_proj_skill))
        .route("/v1/skills/{proj_id}", get(list_proj_skills))
}
