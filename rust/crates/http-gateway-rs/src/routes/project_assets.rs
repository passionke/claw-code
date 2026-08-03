//! project_assets routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::{
    download_project_skill_archive, get_effective_prompt, get_proj_skill, get_project_claude_md,
    get_project_skill_tree, get_project_tools_catalog, list_proj_skills, post_effective_prompt,
    put_project_skill_files, update_project_claude_md, upload_project_skill_archive,
    upsert_project_skill,
};
use axum::routing::{get, post, put};
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/project/claude/{proj_id}",
            get(get_project_claude_md).post(update_project_claude_md),
        )
        .route("/v1/project/skills/{proj_id}", post(upsert_project_skill))
        .route(
            "/v1/project/skills/{proj_id}/archive",
            post(upload_project_skill_archive),
        )
        .route(
            "/v1/project/skills/{proj_id}/{skill_name}/tree",
            get(get_project_skill_tree),
        )
        .route(
            "/v1/project/skills/{proj_id}/{skill_name}/files",
            put(put_project_skill_files),
        )
        .route(
            "/v1/project/skills/{proj_id}/{skill_name}/archive",
            get(download_project_skill_archive),
        )
        .route(
            "/v1/project/prompt/{proj_id}/effective",
            get(get_effective_prompt).post(post_effective_prompt),
        )
        .route("/v1/project/tools/catalog", get(get_project_tools_catalog))
        .route("/v1/skills/{proj_id}/{skill_name}", get(get_proj_skill))
        .route("/v1/skills/{proj_id}", get(list_proj_skills))
}
