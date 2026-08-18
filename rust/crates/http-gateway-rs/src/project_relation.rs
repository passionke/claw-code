//! Unified project-to-project relationship registry for hard platform constraints. Author: kejiqing

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::types::Json;
use sqlx::{Error as SqlxError, Row};

use crate::session_db::GatewaySessionDb;

pub const RELATION_TYPE_ROUTER_DELEGATE: &str = "router_delegate";
pub const RELATION_TYPE_MASTER_APPRENTICE: &str = "master_apprentice";
pub const RELATION_TYPE_MASTER_OBSERVATION: &str = "master_observation";

fn normalize_relation_type(raw: &str) -> Result<&str, String> {
    match raw.trim() {
        RELATION_TYPE_ROUTER_DELEGATE
        | RELATION_TYPE_MASTER_APPRENTICE
        | RELATION_TYPE_MASTER_OBSERVATION => Ok(raw.trim()),
        other => Err(format!(
            "invalid relation_type={other:?}; expected router_delegate|master_apprentice|master_observation"
        )),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRelationRow {
    pub relation_type: String,
    pub from_proj_id: i64,
    pub to_proj_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation_label: Option<String>,
    #[serde(default)]
    pub relation_meta_json: Value,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectReferenceRow {
    pub relation_type: String,
    pub ref_by_proj_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation_label: Option<String>,
    #[serde(default)]
    pub relation_meta_json: Value,
}

impl GatewaySessionDb {
    pub async fn upsert_project_relation(&self, row: &ProjectRelationRow) -> Result<(), SqlxError> {
        let relation_type = normalize_relation_type(&row.relation_type)
            .map_err(|e| SqlxError::Configuration(e.into()))?;
        if row.from_proj_id < 1 || row.to_proj_id < 1 {
            return Err(SqlxError::Configuration(
                "project_relation proj ids must be >= 1".into(),
            ));
        }
        if row.from_proj_id == row.to_proj_id {
            return Err(SqlxError::Configuration(
                "project_relation must not self-reference".into(),
            ));
        }
        sqlx::query(
            r"INSERT INTO project_relation (
                cluster_id, relation_type, from_proj_id, to_proj_id, relation_label,
                relation_meta_json, created_at_ms, updated_at_ms
              ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
              ON CONFLICT (cluster_id, relation_type, from_proj_id, to_proj_id) DO UPDATE SET
                relation_label = EXCLUDED.relation_label,
                relation_meta_json = EXCLUDED.relation_meta_json,
                updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(self.cluster_id())
        .bind(relation_type)
        .bind(row.from_proj_id)
        .bind(row.to_proj_id)
        .bind(row.relation_label.as_deref())
        .bind(Json(&row.relation_meta_json))
        .bind(row.created_at_ms)
        .bind(row.updated_at_ms)
        .execute(self.pg_pool())
        .await?;
        Ok(())
    }

    pub async fn delete_project_relation(
        &self,
        relation_type: &str,
        from_proj_id: i64,
        to_proj_id: i64,
    ) -> Result<bool, SqlxError> {
        let relation_type = normalize_relation_type(relation_type)
            .map_err(|e| SqlxError::Configuration(e.into()))?;
        let r = sqlx::query(
            r"DELETE FROM project_relation
              WHERE cluster_id = $1 AND relation_type = $2 AND from_proj_id = $3 AND to_proj_id = $4",
        )
        .bind(self.cluster_id())
        .bind(relation_type)
        .bind(from_proj_id)
        .bind(to_proj_id)
        .execute(self.pg_pool())
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn replace_project_relations_for_source(
        &self,
        relation_type: &str,
        from_proj_id: i64,
        rows: &[ProjectRelationRow],
    ) -> Result<(), SqlxError> {
        let relation_type = normalize_relation_type(relation_type)
            .map_err(|e| SqlxError::Configuration(e.into()))?;
        let mut tx = self.pg_pool().begin().await?;
        sqlx::query(
            r"DELETE FROM project_relation
              WHERE cluster_id = $1 AND relation_type = $2 AND from_proj_id = $3",
        )
        .bind(self.cluster_id())
        .bind(relation_type)
        .bind(from_proj_id)
        .execute(&mut *tx)
        .await?;
        for row in rows {
            let row_relation_type = normalize_relation_type(&row.relation_type)
                .map_err(|e| SqlxError::Configuration(e.into()))?;
            if row_relation_type != relation_type {
                return Err(SqlxError::Configuration(
                    "replace_project_relations_for_source mixed relation_type".into(),
                ));
            }
            if row.from_proj_id != from_proj_id {
                return Err(SqlxError::Configuration(
                    "replace_project_relations_for_source mixed from_proj_id".into(),
                ));
            }
            if row.from_proj_id == row.to_proj_id {
                return Err(SqlxError::Configuration(
                    "project_relation must not self-reference".into(),
                ));
            }
            sqlx::query(
                r"INSERT INTO project_relation (
                    cluster_id, relation_type, from_proj_id, to_proj_id, relation_label,
                    relation_meta_json, created_at_ms, updated_at_ms
                  ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            )
            .bind(self.cluster_id())
            .bind(relation_type)
            .bind(row.from_proj_id)
            .bind(row.to_proj_id)
            .bind(row.relation_label.as_deref())
            .bind(Json(&row.relation_meta_json))
            .bind(row.created_at_ms)
            .bind(row.updated_at_ms)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_project_references(
        &self,
        proj_id: i64,
    ) -> Result<Vec<ProjectReferenceRow>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT relation_type, from_proj_id, relation_label, relation_meta_json
              FROM project_relation
              WHERE cluster_id = $1 AND to_proj_id = $2
              ORDER BY relation_type, from_proj_id",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .fetch_all(self.pg_pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let meta: Option<Json<Value>> = r.get("relation_meta_json");
                ProjectReferenceRow {
                    relation_type: r.get("relation_type"),
                    ref_by_proj_id: r.get("from_proj_id"),
                    relation_label: r.get("relation_label"),
                    relation_meta_json: meta.map(|Json(v)| v).unwrap_or_else(|| json!({})),
                }
            })
            .collect())
    }

    pub async fn assert_project_deletable(&self, proj_id: i64) -> Result<(), String> {
        let refs = self
            .list_project_references(proj_id)
            .await
            .map_err(|e| e.to_string())?;
        if refs.is_empty() {
            return Ok(());
        }
        let parts: Vec<String> = refs
            .iter()
            .map(|r| {
                format!(
                    "relationType={} refByProjId={}",
                    r.relation_type, r.ref_by_proj_id
                )
            })
            .collect();
        Err(format!(
            "project {proj_id} is still referenced; {}",
            parts.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::master_observer::ProjectMasterLinkRow;
    use crate::session_db::connect_gateway_test_db;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0_i64, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }

    fn ephemeral_test_proj_id() -> i64 {
        i64::try_from(uuid::Uuid::new_v4().as_u128() % 900_000_000).unwrap_or(42) + 1
    }

    async fn ensure_project_row(db: &GatewaySessionDb, proj_id: i64, role: &str) {
        let t = now_ms();
        let project_code = format!("p{proj_id}");
        db.upsert_project_config(crate::session_db::ProjectConfigUpsert {
            proj_id,
            content_rev: "test_rev",
            stable_content_rev: Some("test_rev"),
            draft_open: false,
            updated_at_ms: t,
            rules_json: &json!([]),
            mcp_servers_json: &json!({}),
            skills_sources_json: &json!([]),
            skills_json: &json!([]),
            allowed_tools_json: &json!([]),
            claude_md: Some("# test"),
            git_sync_json: &json!({"enabled": false, "gitRef": "main", "gitUrl": "", "gitTokenSet": false}),
            solve_preflight_json: &json!({"kind": "none"}),
            solve_orchestration_json: &json!({"kind": "single_turn"}),
            language_pipeline_json: &json!({}),
            extra_session_fields_json: &json!([]),
            prompt_limits_json: &json!({}),
            worker_profile_json: &json!({"mode":"strict"}),
            worker_env_json: &json!({}),
            kb_sources_json: &json!([]),
            project_code: &project_code,
            project_description: "test",
            max_iterations: None,
        })
        .await
        .expect("upsert project_config");
        db.set_project_role(proj_id, role).await.expect("set role");
    }

    async fn cleanup_project(db: &GatewaySessionDb, proj_id: i64) {
        let _ = db.delete_project_config(proj_id).await;
        let _ = sqlx::query("DELETE FROM project_relation WHERE cluster_id = $1 AND (from_proj_id = $2 OR to_proj_id = $2)")
            .bind(db.cluster_id())
            .bind(proj_id)
            .execute(db.pg_pool())
            .await;
        let _ = sqlx::query("DELETE FROM project_master_link WHERE cluster_id = $1 AND (master_proj_id = $2 OR apprentice_proj_id = $2 OR observation_proj_id = $2)")
            .bind(db.cluster_id())
            .bind(proj_id)
            .execute(db.pg_pool())
            .await;
    }

    #[tokio::test]
    async fn router_delegate_reference_blocks_delete() {
        let Some(db) = connect_gateway_test_db().await else {
            eprintln!(
                "skip router_delegate_reference_blocks_delete: set CLAW_GATEWAY_TEST_DATABASE_URL"
            );
            return;
        };
        let router = ephemeral_test_proj_id();
        let target = ephemeral_test_proj_id();
        ensure_project_row(&db, router, crate::master_observer::PROJECT_ROLE_ROUTER).await;
        ensure_project_row(&db, target, crate::master_observer::PROJECT_ROLE_NORMAL).await;
        let now = now_ms();
        db.upsert_project_relation(&ProjectRelationRow {
            relation_type: RELATION_TYPE_ROUTER_DELEGATE.into(),
            from_proj_id: router,
            to_proj_id: target,
            relation_label: Some("faq".into()),
            relation_meta_json: json!({"enabled": true}),
            created_at_ms: now,
            updated_at_ms: now,
        })
        .await
        .unwrap();
        let err = db
            .assert_project_deletable(target)
            .await
            .expect_err("delete blocked");
        assert!(err.contains("relationType=router_delegate"), "{err}");
        assert!(err.contains(&format!("refByProjId={router}")), "{err}");
        db.delete_project_relation(RELATION_TYPE_ROUTER_DELEGATE, router, target)
            .await
            .unwrap();
        db.assert_project_deletable(target)
            .await
            .expect("unblocked after relation delete");
        cleanup_project(&db, router).await;
        cleanup_project(&db, target).await;
    }

    #[tokio::test]
    async fn master_link_syncs_project_relations_and_orphan_unblocks_delete() {
        let Some(db) = connect_gateway_test_db().await else {
            eprintln!("skip master_link_syncs_project_relations_and_orphan_unblocks_delete: set CLAW_GATEWAY_TEST_DATABASE_URL");
            return;
        };
        let master = ephemeral_test_proj_id();
        let apprentice = ephemeral_test_proj_id();
        let observation = ephemeral_test_proj_id();
        ensure_project_row(&db, master, crate::master_observer::PROJECT_ROLE_MASTER).await;
        ensure_project_row(&db, apprentice, crate::master_observer::PROJECT_ROLE_NORMAL).await;
        ensure_project_row(
            &db,
            observation,
            crate::master_observer::PROJECT_ROLE_OBSERVATION,
        )
        .await;
        let now = now_ms();
        db.upsert_master_link(&ProjectMasterLinkRow {
            master_proj_id: master,
            apprentice_proj_id: apprentice,
            observation_proj_id: observation,
            apprentice_gateway_base: String::new(),
            apprentice_mcp_token: String::new(),
            mcp_token_set: false,
            orphaned: false,
            created_at_ms: now,
            updated_at_ms: now,
        })
        .await
        .unwrap();
        let apprentice_err = db
            .assert_project_deletable(apprentice)
            .await
            .expect_err("apprentice blocked");
        assert!(
            apprentice_err.contains("relationType=master_apprentice"),
            "{apprentice_err}"
        );
        let obs_err = db
            .assert_project_deletable(observation)
            .await
            .expect_err("observation blocked");
        assert!(
            obs_err.contains("relationType=master_observation"),
            "{obs_err}"
        );
        db.mark_master_link_orphaned(master, apprentice)
            .await
            .unwrap();
        db.assert_project_deletable(apprentice)
            .await
            .expect("apprentice unblocked");
        db.assert_project_deletable(observation)
            .await
            .expect("observation unblocked");
        cleanup_project(&db, master).await;
        cleanup_project(&db, apprentice).await;
        cleanup_project(&db, observation).await;
    }
}
