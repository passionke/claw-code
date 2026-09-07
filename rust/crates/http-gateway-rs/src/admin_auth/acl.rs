//! Pure ACL for human admin principals. Author: kejiqing

use serde::{Deserialize, Serialize};

/// Resolved caller after cass_ session or camt_ token verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPrincipal {
    pub account_id: String,
    pub username: String,
    pub system_admin: bool,
    /// Membership proj_ids (ignored when `system_admin`).
    pub project_ids: Vec<i64>,
    /// True when principal came from a legacy camt_ without accountId (transition).
    #[serde(default)]
    pub legacy_unbound_camt: bool,
}

impl AuthPrincipal {
    #[must_use]
    pub fn system_admin(account_id: impl Into<String>, username: impl Into<String>) -> Self {
        Self {
            account_id: account_id.into(),
            username: username.into(),
            system_admin: true,
            project_ids: Vec::new(),
            legacy_unbound_camt: false,
        }
    }

    #[must_use]
    pub fn space_admin(
        account_id: impl Into<String>,
        username: impl Into<String>,
        project_ids: Vec<i64>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            username: username.into(),
            system_admin: false,
            project_ids,
            legacy_unbound_camt: false,
        }
    }

    /// Transition: camt_ without accountId acts as system_admin scope.
    #[must_use]
    pub fn legacy_camt_system_admin() -> Self {
        Self {
            account_id: String::new(),
            username: "legacy-camt".into(),
            system_admin: true,
            project_ids: Vec::new(),
            legacy_unbound_camt: true,
        }
    }
}

#[must_use]
pub fn can_manage_global(p: &AuthPrincipal) -> bool {
    p.system_admin
}

#[must_use]
pub fn can_manage_members(p: &AuthPrincipal) -> bool {
    p.system_admin
}

#[must_use]
pub fn can_create_or_delete_project(p: &AuthPrincipal) -> bool {
    p.system_admin
}

#[must_use]
pub fn can_access_project(p: &AuthPrincipal, proj_id: i64) -> bool {
    if p.system_admin {
        return true;
    }
    p.project_ids.contains(&proj_id)
}

/// Filter project ids visible to principal. Author: kejiqing
#[must_use]
pub fn filter_project_ids(p: &AuthPrincipal, all: &[i64]) -> Vec<i64> {
    if p.system_admin {
        return all.to_vec();
    }
    all.iter()
        .copied()
        .filter(|id| p.project_ids.contains(id))
        .collect()
}

/// Keep only projects principal may see (by proj_id field accessor). Author: kejiqing
pub fn filter_projects_for<T, F>(p: &AuthPrincipal, all: Vec<T>, proj_id_of: F) -> Vec<T>
where
    F: Fn(&T) -> i64,
{
    if p.system_admin {
        return all;
    }
    all.into_iter()
        .filter(|item| p.project_ids.contains(&proj_id_of(item)))
        .collect()
}

/// Authorize Admin MCP tool call against principal + optional projId in args.
///
/// - Tools that require a project must pass `proj_id`.
/// - `cross_project` tools without projId are denied for non-system_admin.
pub fn authorize_admin_mcp_tool(
    p: &AuthPrincipal,
    tool_name: &str,
    proj_id: Option<i64>,
    tool_is_cross_project_without_id: bool,
) -> Result<(), String> {
    let _ = tool_name;
    if p.system_admin {
        return Ok(());
    }
    if let Some(pid) = proj_id {
        if can_access_project(p, pid) {
            return Ok(());
        }
        return Err(format!(
            "forbidden: no access to projId={pid} for this account"
        ));
    }
    if tool_is_cross_project_without_id {
        return Err("forbidden: space_admin cannot call cross-project tools without projId".into());
    }
    // Tools with no projId and not marked cross-project (e.g. initialize helpers) — allow.
    Ok(())
}

/// Extract projId from MCP tool arguments (camelCase or snake_case). Author: kejiqing
#[must_use]
pub fn proj_id_from_tool_args(args: &serde_json::Value) -> Option<i64> {
    args.get("projId")
        .or_else(|| args.get("proj_id"))
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
}

/// Tools that act across projects when projId is absent. Author: kejiqing
#[must_use]
pub fn admin_mcp_tool_is_cross_project_without_id(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "gateway_task_get" | "gateway_task_list" | "projects_list"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn system_admin_access_matrix() {
        let p = AuthPrincipal::system_admin("a1", "admin");
        assert!(can_access_project(&p, 1));
        assert!(can_access_project(&p, 99));
        assert!(can_manage_global(&p));
        assert!(can_manage_members(&p));
        assert!(can_create_or_delete_project(&p));
        assert_eq!(filter_project_ids(&p, &[1, 2, 3]), vec![1, 2, 3]);
    }

    #[test]
    fn space_admin_membership_and_denials() {
        let p = AuthPrincipal::space_admin("b1", "bob", vec![1, 3]);
        assert!(can_access_project(&p, 1));
        assert!(can_access_project(&p, 3));
        assert!(!can_access_project(&p, 2));
        assert!(!can_manage_global(&p));
        assert!(!can_manage_members(&p));
        assert!(!can_create_or_delete_project(&p));
        assert_eq!(filter_project_ids(&p, &[1, 2, 3, 4]), vec![1, 3]);
    }

    #[test]
    fn multi_space_union_no_cross_leak() {
        let p = AuthPrincipal::space_admin("c1", "carol", vec![10, 20]);
        assert!(can_access_project(&p, 10));
        assert!(can_access_project(&p, 20));
        assert!(!can_access_project(&p, 15));
    }

    #[test]
    fn legacy_camt_acts_as_system_admin() {
        let p = AuthPrincipal::legacy_camt_system_admin();
        assert!(p.legacy_unbound_camt);
        assert!(can_manage_global(&p));
        assert!(can_access_project(&p, 42));
    }

    #[test]
    fn authorize_mcp_tool_space_admin() {
        let p = AuthPrincipal::space_admin("b1", "bob", vec![1]);
        assert!(authorize_admin_mcp_tool(&p, "project_config_get", Some(1), false).is_ok());
        assert!(authorize_admin_mcp_tool(&p, "project_config_get", Some(2), false).is_err());
        assert!(authorize_admin_mcp_tool(&p, "gateway_solve", None, true).is_err());
        assert!(authorize_admin_mcp_tool(&p, "project_list", None, false).is_ok());
        assert!(authorize_admin_mcp_tool(&p, "ping_helper", None, false).is_ok());
    }

    #[test]
    fn authorize_mcp_tool_system_admin_any() {
        let p = AuthPrincipal::system_admin("a1", "admin");
        assert!(authorize_admin_mcp_tool(&p, "project_config_get", Some(99), false).is_ok());
        assert!(authorize_admin_mcp_tool(&p, "gateway_solve", None, true).is_ok());
    }

    #[test]
    fn proj_id_from_args_variants() {
        assert_eq!(proj_id_from_tool_args(&json!({"projId": 7})), Some(7));
        assert_eq!(proj_id_from_tool_args(&json!({"proj_id": "8"})), Some(8));
        assert_eq!(proj_id_from_tool_args(&json!({})), None);
    }

    #[test]
    fn filter_projects_for_structs() {
        #[derive(Debug, PartialEq)]
        struct P {
            proj_id: i64,
        }
        let p = AuthPrincipal::space_admin("b", "bob", vec![2]);
        let got = filter_projects_for(
            &p,
            vec![P { proj_id: 1 }, P { proj_id: 2 }, P { proj_id: 3 }],
            |x| x.proj_id,
        );
        assert_eq!(got, vec![P { proj_id: 2 }]);
    }
}
