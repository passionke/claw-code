//! Human admin accounts (system_admin / space_admin by proj_id). Author: kejiqing

pub mod acl;
pub mod password;
pub mod principal;
pub mod store;

pub use acl::{
    admin_mcp_tool_is_cross_project_without_id, authorize_admin_mcp_tool, can_access_project,
    can_create_or_delete_project, can_manage_global, can_manage_members, filter_project_ids,
    filter_projects_for, proj_id_from_tool_args, AuthPrincipal,
};
pub(crate) use principal::{
    require_create_project_or_open, require_members_manager, require_principal,
    require_project_access_or_open, require_system_admin, require_system_admin_or_open,
    resolve_optional_principal,
};
pub use store::{
    create_account, delete_project_member, ensure_seed_system_admin, list_accounts, login,
    patch_account, revoke_session_by_token, to_public, upsert_project_member, AccountPublic,
    SESSION_TOKEN_PREFIX,
};
