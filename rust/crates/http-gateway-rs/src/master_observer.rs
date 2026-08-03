//! Master / observation-space pairing, repair runs, and default skills. Author: kejiqing

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::types::Json;
use sqlx::{Error as SqlxError, Row};
use uuid::Uuid;

use crate::project_config_draft::{self, DRAFT_CONTENT_REV};
use crate::session_db::{
    now_ms_for_registry, GatewaySessionDb, ProjectConfigRevisionRow, ProjectConfigRow,
    ProjectConfigUpsert,
};

pub const PROJECT_ROLE_NORMAL: &str = "normal";
pub const PROJECT_ROLE_MASTER: &str = "master";
pub const PROJECT_ROLE_OBSERVATION: &str = "observation";

pub const MASTER_MCP_SERVER_NAME: &str = "claw-master-observer";
pub const MASTER_MCP_HTTP_PATH_PREFIX: &str = "/v1/master";

pub const REPAIR_STATUS_OPENED: &str = "opened";
pub const REPAIR_STATUS_SYNCED: &str = "synced";
pub const REPAIR_STATUS_PATCHED: &str = "patched";
pub const REPAIR_STATUS_REPLAYED: &str = "replayed";
pub const REPAIR_STATUS_ANALYZED: &str = "analyzed";
pub const REPAIR_STATUS_DRAFT_PUSHED: &str = "draft_pushed";
pub const REPAIR_STATUS_ABANDONED: &str = "abandoned";

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMasterLinkRow {
    pub master_proj_id: i64,
    pub apprentice_proj_id: i64,
    pub observation_proj_id: i64,
    pub orphaned: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MasterRepairRunRow {
    pub run_id: String,
    pub master_proj_id: i64,
    pub apprentice_proj_id: i64,
    pub observation_proj_id: i64,
    pub master_session_id: Option<String>,
    pub master_turn_id: Option<String>,
    pub status: String,
    pub inventory_json: Value,
    pub baseline_apprentice_content_rev: Option<String>,
    pub observation_content_rev_before: Option<String>,
    pub observation_content_rev_after: Option<String>,
    pub replay_session_ids: Value,
    pub analysis_json: Value,
    pub promote_status: String,
    pub apprentice_draft_note: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GatewayScheduledJobRow {
    pub job_id: String,
    pub master_proj_id: i64,
    pub schedule_kind: String,
    pub run_at_hhmm: String,
    pub weekday: Option<i32>,
    pub enabled: bool,
    pub prompt_template: String,
    pub last_run_at_ms: Option<i64>,
    pub last_task_id: Option<String>,
    pub last_error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Zero-pool worker profile for **observation** projects (on-demand strict). Author: kejiqing
#[must_use]
pub fn zero_pool_worker_profile_json() -> Value {
    json!({"mode": "strict", "poolSize": 0})
}

/// Default worker profile when promoting a project to **master**. Author: kejiqing
#[must_use]
pub fn master_default_worker_profile_json() -> Value {
    json!({"mode": "relaxed"})
}

#[must_use]
pub fn master_claude_md(proj_id: i64) -> String {
    format!(
        r"# Master Observer (proj_{proj_id})

You are a **master observer agent**. You observe paired apprentice projects via the
`claw-master-observer` MCP tools. You do **not** run apprentice business skills yourself.

## Hard rules
- All side effects go through master MCP tools only.
- Quality repair **must** follow skill `master-quality-repair` and the `master_repair_run` state machine.
- Never claim you activated apprentice production config; promote only writes apprentice **draft**.
- When reading apprentice config for repair baselines, use **stable** (effective) revision only.
"
    )
}

#[must_use]
pub fn master_daily_digest_skill() -> Value {
    json!({
            "skillName": "master-daily-digest",
            "skillContent": r"# master-daily-digest

Summarize paired apprentices' Q&A for a time window.

## Steps
1. Call `apprentice_list`.
2. For each apprentice, call `apprentice_sessions_query` with the window from the user prompt.
3. Skim turns/tools/transcripts; produce a concise Chinese report: volume, topics, failure patterns, notable good answers.
4. Do **not** open a repair_run unless the user also asks for quality repair.
",
            "enabled": true
        })
}

#[must_use]
pub fn master_quality_repair_skill() -> Value {
    json!({
        "skillName": "master-quality-repair",
        "skillContent": r#"# master-quality-repair

Extract a replayable issue inventory, patch the observation space, replay, analyze, then promote draft.

## Mandatory MCP sequence (no skipping)
1. `apprentice_sessions_query` — gather candidate turns.
2. `repair_run_open` → status `opened`.
3. `inventory_put` — structured items with sourceSessionId, sourceTurnId, bizdate, replay=true.
4. `observation_sync_from_apprentice` → `synced`.
5. Edit observation config via `observation_config_put_draft` / commit / activate → `patched`.
6. `observation_replay` with run_id → `replayed`.
7. `replay_results_get` then `repair_run_analyze` → `analyzed`.
8. If good enough: `promote_to_apprentice_draft` → `draft_pushed` (never activate).

## Quality criteria
Define what "good" means for this domain in your analysis_json. Prefer minimal skill/CLAUDE.md patches over broad rewrites.
"#,
        "enabled": true
    })
}

#[must_use]
pub fn master_seed_skills_json() -> Value {
    json!([master_daily_digest_skill(), master_quality_repair_skill()])
}

/// Shared bearer for master MCP (worker → gateway). Author: kejiqing
#[must_use]
pub fn master_mcp_shared_token() -> Option<String> {
    std::env::var("CLAW_MASTER_MCP_TOKEN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[must_use]
pub fn master_mcp_url(gateway_base: &str, master_proj_id: i64) -> String {
    let base = gateway_base.trim_end_matches('/');
    format!("{base}{MASTER_MCP_HTTP_PATH_PREFIX}/{master_proj_id}/mcp")
}

/// Inject managed master MCP into settings.mcpServers (sidecar merge). Author: kejiqing
pub fn merge_master_mcp_into_settings(
    settings: &mut Value,
    master_proj_id: i64,
    gateway_base: &str,
    token: &str,
) {
    let url = master_mcp_url(gateway_base, master_proj_id);
    let entry = json!({
        "type": "streamable-http",
        "url": url,
        "headers": {
            "Authorization": format!("Bearer {token}")
        }
    });
    let Some(obj) = settings.as_object_mut() else {
        return;
    };
    let servers = obj
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut();
    if let Some(servers) = servers {
        servers.insert(MASTER_MCP_SERVER_NAME.to_string(), entry);
    }
}

pub fn validate_project_role(role: &str) -> Result<&str, String> {
    match role.trim() {
        PROJECT_ROLE_NORMAL | PROJECT_ROLE_MASTER | PROJECT_ROLE_OBSERVATION => Ok(role.trim()),
        other => Err(format!(
            "invalid project_role={other:?}; expected normal|master|observation"
        )),
    }
}

/// Legal repair_run status transitions (plus abandon from any non-terminal). Author: kejiqing
#[allow(clippy::unnested_or_patterns)]
pub fn can_transition_repair_status(from: &str, to: &str) -> bool {
    if to == REPAIR_STATUS_ABANDONED {
        return from != REPAIR_STATUS_DRAFT_PUSHED && from != REPAIR_STATUS_ABANDONED;
    }
    matches!(
        (from, to),
        (REPAIR_STATUS_OPENED, REPAIR_STATUS_SYNCED)
            | (REPAIR_STATUS_SYNCED, REPAIR_STATUS_PATCHED)
            | (REPAIR_STATUS_PATCHED, REPAIR_STATUS_REPLAYED)
            | (REPAIR_STATUS_PATCHED, REPAIR_STATUS_PATCHED)
            | (REPAIR_STATUS_REPLAYED, REPAIR_STATUS_ANALYZED)
            | (REPAIR_STATUS_REPLAYED, REPAIR_STATUS_PATCHED)
            | (REPAIR_STATUS_ANALYZED, REPAIR_STATUS_DRAFT_PUSHED)
            | (REPAIR_STATUS_ANALYZED, REPAIR_STATUS_PATCHED)
    )
}

/// Validate inventory_json shape; replay=true items need session/turn ids. Author: kejiqing
pub fn validate_inventory_json(inv: &Value) -> Result<(), String> {
    let items = inv
        .get("items")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "inventoryJson.items must be an array".to_string())?;
    for (i, item) in items.iter().enumerate() {
        let item_id = item
            .get("itemId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("items[{i}].itemId required"))?;
        if item_id.trim().is_empty() {
            return Err(format!("items[{i}].itemId must be non-empty"));
        }
        let replay = item
            .get("replay")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if replay {
            if item
                .get("sourceSessionId")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_none()
            {
                return Err(format!(
                    "items[{i}] ({item_id}) replay=true requires sourceSessionId"
                ));
            }
            if item
                .get("sourceTurnId")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_none()
            {
                return Err(format!(
                    "items[{i}] ({item_id}) replay=true requires sourceTurnId"
                ));
            }
        }
    }
    Ok(())
}

/// Items with `replay: true` (after [`validate_inventory_json`]). Author: kejiqing
#[must_use]
pub fn replayable_inventory_items(inv: &Value) -> Vec<Value> {
    inv.get("items")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|item| {
            item.get("replay")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

/// Apply schedule prompt placeholders. Author: kejiqing
#[must_use]
pub fn render_schedule_prompt(
    template: &str,
    apprentice_ids: &[i64],
    bizdate_yesterday: &str,
) -> String {
    let ids = apprentice_ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let filled = template
        .replace("{{apprentice_ids}}", &ids)
        .replace("{{bizdate_yesterday}}", bizdate_yesterday);
    if filled.trim().is_empty() {
        format!(
            "执行 skill master-daily-digest，学徒={{{ids}}}，窗口 bizdate={{{bizdate_yesterday}}}"
        )
    } else {
        filled
    }
}

/// Master MCP tool names (tools/list contract). Author: kejiqing
pub const MASTER_MCP_TOOL_NAMES: &[&str] = &[
    "apprentice_list",
    "apprentice_config_get",
    "apprentice_sessions_query",
    "apprentice_turns_list",
    "observation_sync_from_apprentice",
    "observation_config_put_draft",
    "repair_run_open",
    "inventory_put",
    "observation_replay",
    "replay_results_get",
    "repair_run_analyze",
    "promote_to_apprentice_draft",
    "apprentice_config_put_draft",
    "observation_solve",
];

impl GatewaySessionDb {
    pub async fn get_project_role(&self, proj_id: i64) -> Result<String, SqlxError> {
        let role: Option<String> = sqlx::query_scalar(
            "SELECT project_role FROM project_config WHERE cluster_id = $1 AND proj_id = $2",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .fetch_optional(self.pg_pool())
        .await?;
        Ok(role.unwrap_or_else(|| PROJECT_ROLE_NORMAL.to_string()))
    }

    pub async fn set_project_role(&self, proj_id: i64, role: &str) -> Result<(), SqlxError> {
        let role = validate_project_role(role).map_err(|e| SqlxError::Configuration(e.into()))?;
        let r = sqlx::query(
            "UPDATE project_config SET project_role = $3, updated_at_ms = $4 \
             WHERE cluster_id = $1 AND proj_id = $2",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .bind(role)
        .bind(now_ms_for_registry())
        .execute(self.pg_pool())
        .await?;
        if r.rows_affected() == 0 {
            return Err(SqlxError::RowNotFound);
        }
        Ok(())
    }

    pub async fn list_master_links(
        &self,
        master_proj_id: i64,
    ) -> Result<Vec<ProjectMasterLinkRow>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT master_proj_id, apprentice_proj_id, observation_proj_id, orphaned,
                     created_at_ms, updated_at_ms
              FROM project_master_link
              WHERE cluster_id = $1 AND master_proj_id = $2
              ORDER BY apprentice_proj_id",
        )
        .bind(self.cluster_id())
        .bind(master_proj_id)
        .fetch_all(self.pg_pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| ProjectMasterLinkRow {
                master_proj_id: r.get("master_proj_id"),
                apprentice_proj_id: r.get("apprentice_proj_id"),
                observation_proj_id: r.get("observation_proj_id"),
                orphaned: r.get("orphaned"),
                created_at_ms: r.get("created_at_ms"),
                updated_at_ms: r.get("updated_at_ms"),
            })
            .collect())
    }

    pub async fn get_master_link(
        &self,
        master_proj_id: i64,
        apprentice_proj_id: i64,
    ) -> Result<Option<ProjectMasterLinkRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT master_proj_id, apprentice_proj_id, observation_proj_id, orphaned,
                     created_at_ms, updated_at_ms
              FROM project_master_link
              WHERE cluster_id = $1 AND master_proj_id = $2 AND apprentice_proj_id = $3",
        )
        .bind(self.cluster_id())
        .bind(master_proj_id)
        .bind(apprentice_proj_id)
        .fetch_optional(self.pg_pool())
        .await?;
        Ok(row.map(|r| ProjectMasterLinkRow {
            master_proj_id: r.get("master_proj_id"),
            apprentice_proj_id: r.get("apprentice_proj_id"),
            observation_proj_id: r.get("observation_proj_id"),
            orphaned: r.get("orphaned"),
            created_at_ms: r.get("created_at_ms"),
            updated_at_ms: r.get("updated_at_ms"),
        }))
    }

    pub async fn upsert_master_link(&self, link: &ProjectMasterLinkRow) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO project_master_link (
                cluster_id, master_proj_id, apprentice_proj_id, observation_proj_id,
                orphaned, created_at_ms, updated_at_ms
              ) VALUES ($1, $2, $3, $4, $5, $6, $7)
              ON CONFLICT (cluster_id, master_proj_id, apprentice_proj_id) DO UPDATE SET
                observation_proj_id = EXCLUDED.observation_proj_id,
                orphaned = EXCLUDED.orphaned,
                updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(self.cluster_id())
        .bind(link.master_proj_id)
        .bind(link.apprentice_proj_id)
        .bind(link.observation_proj_id)
        .bind(link.orphaned)
        .bind(link.created_at_ms)
        .bind(link.updated_at_ms)
        .execute(self.pg_pool())
        .await?;
        Ok(())
    }

    pub async fn mark_master_link_orphaned(
        &self,
        master_proj_id: i64,
        apprentice_proj_id: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "UPDATE project_master_link SET orphaned = TRUE, updated_at_ms = $4 \
             WHERE cluster_id = $1 AND master_proj_id = $2 AND apprentice_proj_id = $3",
        )
        .bind(self.cluster_id())
        .bind(master_proj_id)
        .bind(apprentice_proj_id)
        .bind(now_ms_for_registry())
        .execute(self.pg_pool())
        .await?;
        Ok(())
    }

    pub async fn assert_master_owns_apprentice(
        &self,
        master_proj_id: i64,
        apprentice_proj_id: i64,
    ) -> Result<ProjectMasterLinkRow, String> {
        let link = self
            .get_master_link(master_proj_id, apprentice_proj_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                format!("apprentice {apprentice_proj_id} is not paired to master {master_proj_id}")
            })?;
        if link.orphaned {
            return Err(format!(
                "apprentice {apprentice_proj_id} link is orphaned for master {master_proj_id}"
            ));
        }
        Ok(link)
    }

    pub async fn assert_master_owns_observation(
        &self,
        master_proj_id: i64,
        observation_proj_id: i64,
    ) -> Result<ProjectMasterLinkRow, String> {
        let links = self
            .list_master_links(master_proj_id)
            .await
            .map_err(|e| e.to_string())?;
        links
            .into_iter()
            .find(|l| l.observation_proj_id == observation_proj_id && !l.orphaned)
            .ok_or_else(|| {
                format!(
                    "observation {observation_proj_id} is not paired to master {master_proj_id}"
                )
            })
    }

    pub async fn insert_repair_run(&self, row: &MasterRepairRunRow) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO master_repair_run (
                cluster_id, run_id, master_proj_id, apprentice_proj_id, observation_proj_id,
                master_session_id, master_turn_id, status, inventory_json,
                baseline_apprentice_content_rev, observation_content_rev_before,
                observation_content_rev_after, replay_session_ids, analysis_json,
                promote_status, apprentice_draft_note, created_at_ms, updated_at_ms
              ) VALUES (
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18
              )",
        )
        .bind(self.cluster_id())
        .bind(&row.run_id)
        .bind(row.master_proj_id)
        .bind(row.apprentice_proj_id)
        .bind(row.observation_proj_id)
        .bind(&row.master_session_id)
        .bind(&row.master_turn_id)
        .bind(&row.status)
        .bind(Json(&row.inventory_json))
        .bind(&row.baseline_apprentice_content_rev)
        .bind(&row.observation_content_rev_before)
        .bind(&row.observation_content_rev_after)
        .bind(Json(&row.replay_session_ids))
        .bind(Json(&row.analysis_json))
        .bind(&row.promote_status)
        .bind(&row.apprentice_draft_note)
        .bind(row.created_at_ms)
        .bind(row.updated_at_ms)
        .execute(self.pg_pool())
        .await?;
        Ok(())
    }

    pub async fn get_repair_run(
        &self,
        run_id: &str,
    ) -> Result<Option<MasterRepairRunRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT run_id, master_proj_id, apprentice_proj_id, observation_proj_id,
                     master_session_id, master_turn_id, status, inventory_json,
                     baseline_apprentice_content_rev, observation_content_rev_before,
                     observation_content_rev_after, replay_session_ids, analysis_json,
                     promote_status, apprentice_draft_note, created_at_ms, updated_at_ms
              FROM master_repair_run WHERE cluster_id = $1 AND run_id = $2",
        )
        .bind(self.cluster_id())
        .bind(run_id)
        .fetch_optional(self.pg_pool())
        .await?;
        Ok(row.map(|r| row_to_repair_run(&r)))
    }

    pub async fn list_repair_runs(
        &self,
        master_proj_id: i64,
        limit: i64,
    ) -> Result<Vec<MasterRepairRunRow>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT run_id, master_proj_id, apprentice_proj_id, observation_proj_id,
                     master_session_id, master_turn_id, status, inventory_json,
                     baseline_apprentice_content_rev, observation_content_rev_before,
                     observation_content_rev_after, replay_session_ids, analysis_json,
                     promote_status, apprentice_draft_note, created_at_ms, updated_at_ms
              FROM master_repair_run
              WHERE cluster_id = $1 AND master_proj_id = $2
              ORDER BY created_at_ms DESC
              LIMIT $3",
        )
        .bind(self.cluster_id())
        .bind(master_proj_id)
        .bind(limit)
        .fetch_all(self.pg_pool())
        .await?;
        Ok(rows.iter().map(row_to_repair_run).collect())
    }

    pub async fn update_repair_run(&self, row: &MasterRepairRunRow) -> Result<(), SqlxError> {
        sqlx::query(
            r"UPDATE master_repair_run SET
                status = $3,
                inventory_json = $4,
                baseline_apprentice_content_rev = $5,
                observation_content_rev_before = $6,
                observation_content_rev_after = $7,
                replay_session_ids = $8,
                analysis_json = $9,
                promote_status = $10,
                apprentice_draft_note = $11,
                updated_at_ms = $12
              WHERE cluster_id = $1 AND run_id = $2",
        )
        .bind(self.cluster_id())
        .bind(&row.run_id)
        .bind(&row.status)
        .bind(Json(&row.inventory_json))
        .bind(&row.baseline_apprentice_content_rev)
        .bind(&row.observation_content_rev_before)
        .bind(&row.observation_content_rev_after)
        .bind(Json(&row.replay_session_ids))
        .bind(Json(&row.analysis_json))
        .bind(&row.promote_status)
        .bind(&row.apprentice_draft_note)
        .bind(row.updated_at_ms)
        .execute(self.pg_pool())
        .await?;
        Ok(())
    }

    pub async fn upsert_scheduled_job(
        &self,
        job: &GatewayScheduledJobRow,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_scheduled_job (
                cluster_id, job_id, master_proj_id, schedule_kind, run_at_hhmm, weekday,
                enabled, prompt_template, last_run_at_ms, last_task_id, last_error,
                created_at_ms, updated_at_ms
              ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
              ON CONFLICT (cluster_id, job_id) DO UPDATE SET
                master_proj_id = EXCLUDED.master_proj_id,
                schedule_kind = EXCLUDED.schedule_kind,
                run_at_hhmm = EXCLUDED.run_at_hhmm,
                weekday = EXCLUDED.weekday,
                enabled = EXCLUDED.enabled,
                prompt_template = EXCLUDED.prompt_template,
                last_run_at_ms = EXCLUDED.last_run_at_ms,
                last_task_id = EXCLUDED.last_task_id,
                last_error = EXCLUDED.last_error,
                updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(self.cluster_id())
        .bind(&job.job_id)
        .bind(job.master_proj_id)
        .bind(&job.schedule_kind)
        .bind(&job.run_at_hhmm)
        .bind(job.weekday)
        .bind(job.enabled)
        .bind(&job.prompt_template)
        .bind(job.last_run_at_ms)
        .bind(&job.last_task_id)
        .bind(&job.last_error)
        .bind(job.created_at_ms)
        .bind(job.updated_at_ms)
        .execute(self.pg_pool())
        .await?;
        Ok(())
    }

    pub async fn list_scheduled_jobs(
        &self,
        master_proj_id: Option<i64>,
    ) -> Result<Vec<GatewayScheduledJobRow>, SqlxError> {
        let rows = if let Some(mid) = master_proj_id {
            sqlx::query(
                r"SELECT job_id, master_proj_id, schedule_kind, run_at_hhmm, weekday, enabled,
                         prompt_template, last_run_at_ms, last_task_id, last_error,
                         created_at_ms, updated_at_ms
                  FROM gateway_scheduled_job
                  WHERE cluster_id = $1 AND master_proj_id = $2
                  ORDER BY job_id",
            )
            .bind(self.cluster_id())
            .bind(mid)
            .fetch_all(self.pg_pool())
            .await?
        } else {
            sqlx::query(
                r"SELECT job_id, master_proj_id, schedule_kind, run_at_hhmm, weekday, enabled,
                         prompt_template, last_run_at_ms, last_task_id, last_error,
                         created_at_ms, updated_at_ms
                  FROM gateway_scheduled_job
                  WHERE cluster_id = $1
                  ORDER BY master_proj_id, job_id",
            )
            .bind(self.cluster_id())
            .fetch_all(self.pg_pool())
            .await?
        };
        Ok(rows.iter().map(row_to_scheduled_job).collect())
    }

    pub async fn list_enabled_scheduled_jobs(
        &self,
    ) -> Result<Vec<GatewayScheduledJobRow>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT job_id, master_proj_id, schedule_kind, run_at_hhmm, weekday, enabled,
                     prompt_template, last_run_at_ms, last_task_id, last_error,
                     created_at_ms, updated_at_ms
              FROM gateway_scheduled_job
              WHERE cluster_id = $1 AND enabled = TRUE
              ORDER BY job_id",
        )
        .bind(self.cluster_id())
        .fetch_all(self.pg_pool())
        .await?;
        Ok(rows.iter().map(row_to_scheduled_job).collect())
    }

    pub async fn delete_scheduled_job(&self, job_id: &str) -> Result<bool, SqlxError> {
        let r =
            sqlx::query("DELETE FROM gateway_scheduled_job WHERE cluster_id = $1 AND job_id = $2")
                .bind(self.cluster_id())
                .bind(job_id)
                .execute(self.pg_pool())
                .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn get_turn_for_replay(
        &self,
        session_id: &str,
        proj_id: i64,
        turn_id: &str,
    ) -> Result<Option<(Option<String>, Option<Value>)>, SqlxError> {
        let row = sqlx::query(
            r"SELECT user_prompt, entry_params_json
              FROM gateway_turns
              WHERE cluster_id = $1 AND session_id = $2 AND proj_id = $3 AND turn_id = $4",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .bind(turn_id)
        .fetch_optional(self.pg_pool())
        .await?;
        Ok(row.map(|r| {
            let prompt: Option<String> = r.get("user_prompt");
            let entry: Option<Json<Value>> = r.get("entry_params_json");
            (prompt, entry.map(|Json(v)| v))
        }))
    }
}

fn row_to_repair_run(r: &sqlx::postgres::PgRow) -> MasterRepairRunRow {
    MasterRepairRunRow {
        run_id: r.get("run_id"),
        master_proj_id: r.get("master_proj_id"),
        apprentice_proj_id: r.get("apprentice_proj_id"),
        observation_proj_id: r.get("observation_proj_id"),
        master_session_id: r.get("master_session_id"),
        master_turn_id: r.get("master_turn_id"),
        status: r.get("status"),
        inventory_json: r
            .try_get::<Json<Value>, _>("inventory_json")
            .map(|j| j.0)
            .unwrap_or_else(|_| json!({"items":[]})),
        baseline_apprentice_content_rev: r.get("baseline_apprentice_content_rev"),
        observation_content_rev_before: r.get("observation_content_rev_before"),
        observation_content_rev_after: r.get("observation_content_rev_after"),
        replay_session_ids: r
            .try_get::<Json<Value>, _>("replay_session_ids")
            .map(|j| j.0)
            .unwrap_or_else(|_| json!([])),
        analysis_json: r
            .try_get::<Json<Value>, _>("analysis_json")
            .map(|j| j.0)
            .unwrap_or_else(|_| json!({})),
        promote_status: r.get("promote_status"),
        apprentice_draft_note: r.get("apprentice_draft_note"),
        created_at_ms: r.get("created_at_ms"),
        updated_at_ms: r.get("updated_at_ms"),
    }
}

fn row_to_scheduled_job(r: &sqlx::postgres::PgRow) -> GatewayScheduledJobRow {
    GatewayScheduledJobRow {
        job_id: r.get("job_id"),
        master_proj_id: r.get("master_proj_id"),
        schedule_kind: r.get("schedule_kind"),
        run_at_hhmm: r.get("run_at_hhmm"),
        weekday: r.get("weekday"),
        enabled: r.get("enabled"),
        prompt_template: r.get("prompt_template"),
        last_run_at_ms: r.get("last_run_at_ms"),
        last_task_id: r.get("last_task_id"),
        last_error: r.get("last_error"),
        created_at_ms: r.get("created_at_ms"),
        updated_at_ms: r.get("updated_at_ms"),
    }
}

/// Copy package fields from source stable row onto target (formal activate). Author: kejiqing
pub async fn clone_stable_config_onto_project(
    db: &GatewaySessionDb,
    source: &ProjectConfigRow,
    target_proj_id: i64,
    project_code: &str,
    project_description: &str,
    worker_profile_json: &Value,
    claude_md_override: Option<&str>,
    skills_json_override: Option<&Value>,
) -> Result<String, String> {
    let now = now_ms_for_registry();
    let content_rev = project_config_draft::format_formal_content_rev_local_ms(now);
    let claude = claude_md_override
        .map(str::to_string)
        .or_else(|| source.claude_md.clone());
    let skills = skills_json_override
        .cloned()
        .unwrap_or_else(|| source.skills_json.clone());
    let empty_sources = json!([]);
    db.upsert_project_config(ProjectConfigUpsert {
        proj_id: target_proj_id,
        content_rev: &content_rev,
        stable_content_rev: Some(content_rev.as_str()),
        draft_open: false,
        updated_at_ms: now,
        rules_json: &source.rules_json,
        mcp_servers_json: &source.mcp_servers_json,
        skills_sources_json: &empty_sources,
        skills_json: &skills,
        allowed_tools_json: &source.allowed_tools_json,
        claude_md: claude.as_deref(),
        git_sync_json: &json!({}),
        solve_preflight_json: &source.solve_preflight_json,
        solve_orchestration_json: &source.solve_orchestration_json,
        language_pipeline_json: &source.language_pipeline_json,
        extra_session_fields_json: &source.extra_session_fields_json,
        prompt_limits_json: &source.prompt_limits_json,
        worker_profile_json,
        worker_env_json: &source.worker_env_json,
        project_code,
        project_description,
        max_iterations: source.max_iterations,
    })
    .await
    .map_err(|e| e.to_string())?;

    let rev = ProjectConfigRevisionRow {
        proj_id: target_proj_id,
        content_rev: content_rev.clone(),
        created_at_ms: now,
        note: Some("cloned from apprentice stable / master seed".into()),
        rules_json: source.rules_json.clone(),
        mcp_servers_json: source.mcp_servers_json.clone(),
        skills_sources_json: empty_sources,
        skills_json: skills,
        allowed_tools_json: source.allowed_tools_json.clone(),
        claude_md: claude,
    };
    let _ = db
        .insert_project_config_revision_immutable(&rev)
        .await
        .map_err(|e| e.to_string())?;
    Ok(content_rev)
}

/// Apply master seed CLAUDE + skills onto an existing project and set role. Author: kejiqing
pub async fn seed_master_project(db: &GatewaySessionDb, proj_id: i64) -> Result<(), String> {
    let row = db
        .get_project_config(proj_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project {proj_id} not found"))?;
    let role = db
        .get_project_role(proj_id)
        .await
        .map_err(|e| e.to_string())?;
    if role == PROJECT_ROLE_OBSERVATION {
        return Err("cannot set observation project as master".into());
    }
    let now = now_ms_for_registry();
    let content_rev = project_config_draft::format_formal_content_rev_local_ms(now);
    let claude = master_claude_md(proj_id);
    let skills = master_seed_skills_json();
    let empty_sources = json!([]);
    let worker = master_default_worker_profile_json();
    db.upsert_project_config(ProjectConfigUpsert {
        proj_id,
        content_rev: &content_rev,
        stable_content_rev: Some(content_rev.as_str()),
        draft_open: false,
        updated_at_ms: now,
        rules_json: &row.rules_json,
        mcp_servers_json: &row.mcp_servers_json,
        skills_sources_json: &empty_sources,
        skills_json: &skills,
        allowed_tools_json: &row.allowed_tools_json,
        claude_md: Some(&claude),
        git_sync_json: &row.git_sync_json,
        solve_preflight_json: &row.solve_preflight_json,
        solve_orchestration_json: &row.solve_orchestration_json,
        language_pipeline_json: &row.language_pipeline_json,
        extra_session_fields_json: &row.extra_session_fields_json,
        prompt_limits_json: &row.prompt_limits_json,
        worker_profile_json: &worker,
        worker_env_json: &row.worker_env_json,
        project_code: &row.project_code,
        project_description: &row.project_description,
        max_iterations: row.max_iterations,
    })
    .await
    .map_err(|e| e.to_string())?;
    let rev = ProjectConfigRevisionRow {
        proj_id,
        content_rev: content_rev.clone(),
        created_at_ms: now,
        note: Some("master role seed".into()),
        rules_json: row.rules_json,
        mcp_servers_json: row.mcp_servers_json,
        skills_sources_json: empty_sources,
        skills_json: skills,
        allowed_tools_json: row.allowed_tools_json,
        claude_md: Some(claude),
    };
    let _ = db
        .insert_project_config_revision_immutable(&rev)
        .await
        .map_err(|e| e.to_string())?;
    db.set_project_role(proj_id, PROJECT_ROLE_MASTER)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn new_repair_run_id() -> String {
    format!("mrr_{}", Uuid::new_v4().simple())
}

pub fn new_scheduled_job_id() -> String {
    format!("gsj_{}", Uuid::new_v4().simple())
}

/// Push observation formal package into apprentice `__draft__` (no activate). Author: kejiqing
pub async fn promote_observation_to_apprentice_draft(
    db: &GatewaySessionDb,
    observation_proj_id: i64,
    apprentice_proj_id: i64,
    note: &str,
) -> Result<(), String> {
    let obs = project_config_draft::row_for_materialize(db, observation_proj_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("observation {observation_proj_id} missing"))?;
    let mut apprentice = project_config_draft::ensure_draft(db, apprentice_proj_id)
        .await
        .map_err(|e| e.to_string())?;
    apprentice.rules_json = obs.rules_json;
    apprentice.mcp_servers_json = obs.mcp_servers_json;
    apprentice.skills_json = obs.skills_json;
    apprentice.allowed_tools_json = obs.allowed_tools_json;
    apprentice.claude_md = obs.claude_md;
    apprentice.content_rev = DRAFT_CONTENT_REV.to_string();
    apprentice.draft_open = true;
    apprentice.updated_at_ms = now_ms_for_registry();
    let _ = note;
    db.upsert_project_config(ProjectConfigUpsert {
        proj_id: apprentice.proj_id,
        content_rev: DRAFT_CONTENT_REV,
        stable_content_rev: apprentice.stable_content_rev.as_deref(),
        draft_open: true,
        updated_at_ms: apprentice.updated_at_ms,
        rules_json: &apprentice.rules_json,
        mcp_servers_json: &apprentice.mcp_servers_json,
        skills_sources_json: &apprentice.skills_sources_json,
        skills_json: &apprentice.skills_json,
        allowed_tools_json: &apprentice.allowed_tools_json,
        claude_md: apprentice.claude_md.as_deref(),
        git_sync_json: &apprentice.git_sync_json,
        solve_preflight_json: &apprentice.solve_preflight_json,
        solve_orchestration_json: &apprentice.solve_orchestration_json,
        language_pipeline_json: &apprentice.language_pipeline_json,
        extra_session_fields_json: &apprentice.extra_session_fields_json,
        prompt_limits_json: &apprentice.prompt_limits_json,
        worker_profile_json: &apprentice.worker_profile_json,
        worker_env_json: &apprentice.worker_env_json,
        project_code: &apprentice.project_code,
        project_description: &apprentice.project_description,
        max_iterations: apprentice.max_iterations,
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::test_env_lock;
    use serde_json::json;

    #[test]
    fn validate_project_role_accepts_known() {
        assert_eq!(validate_project_role("normal").unwrap(), "normal");
        assert_eq!(validate_project_role(" master ").unwrap(), "master");
        assert_eq!(validate_project_role("observation").unwrap(), "observation");
        assert!(validate_project_role("boss").is_err());
        assert!(validate_project_role("").is_err());
    }

    #[test]
    fn repair_transition_matrix_happy_path() {
        let path = [
            (REPAIR_STATUS_OPENED, REPAIR_STATUS_SYNCED),
            (REPAIR_STATUS_SYNCED, REPAIR_STATUS_PATCHED),
            (REPAIR_STATUS_PATCHED, REPAIR_STATUS_REPLAYED),
            (REPAIR_STATUS_REPLAYED, REPAIR_STATUS_ANALYZED),
            (REPAIR_STATUS_ANALYZED, REPAIR_STATUS_DRAFT_PUSHED),
        ];
        for (from, to) in path {
            assert!(
                can_transition_repair_status(from, to),
                "expected {from} -> {to}"
            );
        }
    }

    #[test]
    fn repair_transition_rejects_skips_and_terminal_abandon() {
        assert!(!can_transition_repair_status(
            REPAIR_STATUS_OPENED,
            REPAIR_STATUS_PATCHED
        ));
        assert!(!can_transition_repair_status(
            REPAIR_STATUS_OPENED,
            REPAIR_STATUS_DRAFT_PUSHED
        ));
        assert!(!can_transition_repair_status(
            REPAIR_STATUS_DRAFT_PUSHED,
            REPAIR_STATUS_ABANDONED
        ));
        assert!(!can_transition_repair_status(
            REPAIR_STATUS_ABANDONED,
            REPAIR_STATUS_ABANDONED
        ));
        assert!(can_transition_repair_status(
            REPAIR_STATUS_PATCHED,
            REPAIR_STATUS_ABANDONED
        ));
        // retry patch / re-replay loops
        assert!(can_transition_repair_status(
            REPAIR_STATUS_PATCHED,
            REPAIR_STATUS_PATCHED
        ));
        assert!(can_transition_repair_status(
            REPAIR_STATUS_REPLAYED,
            REPAIR_STATUS_PATCHED
        ));
        assert!(can_transition_repair_status(
            REPAIR_STATUS_ANALYZED,
            REPAIR_STATUS_PATCHED
        ));
    }

    #[test]
    fn inventory_requires_items_array_and_item_id() {
        assert!(validate_inventory_json(&json!({})).is_err());
        assert!(validate_inventory_json(&json!({"items": "x"})).is_err());
        assert!(validate_inventory_json(&json!({"items": [{}]})).is_err());
        assert!(validate_inventory_json(&json!({"items": [{"itemId": ""}]})).is_err());
        assert!(validate_inventory_json(&json!({"items": [{"itemId": "i1"}]})).is_ok());
    }

    #[test]
    fn inventory_replay_true_requires_source_ids() {
        let missing_session = json!({"items":[{
            "itemId":"i1","replay":true,"sourceTurnId":"t1"
        }]});
        assert!(validate_inventory_json(&missing_session)
            .unwrap_err()
            .contains("sourceSessionId"));
        let missing_turn = json!({"items":[{
            "itemId":"i1","replay":true,"sourceSessionId":"s1"
        }]});
        assert!(validate_inventory_json(&missing_turn)
            .unwrap_err()
            .contains("sourceTurnId"));
        let ok = json!({"items":[{
            "itemId":"i1",
            "replay":true,
            "sourceSessionId":"s1",
            "sourceTurnId":"t1",
            "bizdate":"20260802"
        }]});
        assert!(validate_inventory_json(&ok).is_ok());
    }

    #[test]
    fn replayable_inventory_filters_replay_flag() {
        let inv = json!({"items":[
            {"itemId":"a","replay":true,"sourceSessionId":"s","sourceTurnId":"t"},
            {"itemId":"b","replay":false},
            {"itemId":"c"}
        ]});
        let items = replayable_inventory_items(&inv);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["itemId"], "a");
    }

    #[test]
    fn master_mcp_url_and_settings_merge() {
        assert_eq!(
            master_mcp_url("http://gw:8080/", 7),
            "http://gw:8080/v1/master/7/mcp"
        );
        let mut settings = json!({"mcpServers": {"other": {"url": "x"}}});
        merge_master_mcp_into_settings(&mut settings, 3, "http://gw", "tok");
        let servers = settings["mcpServers"].as_object().unwrap();
        assert!(servers.contains_key("other"));
        let entry = &servers[MASTER_MCP_SERVER_NAME];
        assert_eq!(entry["type"], "streamable-http");
        assert_eq!(entry["url"], "http://gw/v1/master/3/mcp");
        assert_eq!(entry["headers"]["Authorization"], "Bearer tok");
    }

    #[test]
    fn master_seed_skills_and_claude_mention_state_machine() {
        let skills = master_seed_skills_json();
        let arr = skills.as_array().unwrap();
        let names: Vec<&str> = arr
            .iter()
            .filter_map(|s| s.get("skillName").and_then(|v| v.as_str()))
            .collect();
        assert!(names.contains(&"master-daily-digest"));
        assert!(names.contains(&"master-quality-repair"));
        let repair = arr
            .iter()
            .find(|s| s["skillName"] == "master-quality-repair")
            .unwrap();
        let content = repair["skillContent"].as_str().unwrap();
        assert!(content.contains("repair_run_open"));
        assert!(content.contains("inventory_put"));
        assert!(content.contains("observation_replay"));
        assert!(content.contains("promote_to_apprentice_draft"));
        let claude = master_claude_md(9);
        assert!(claude.contains("proj_9"));
        assert!(claude.contains("master-quality-repair"));
        assert!(claude.contains("draft"));
    }

    #[test]
    fn zero_pool_profile_and_tool_names_contract() {
        let p = zero_pool_worker_profile_json();
        assert_eq!(p["mode"], "strict");
        assert_eq!(p["poolSize"], 0);
        let m = master_default_worker_profile_json();
        assert_eq!(m["mode"], "relaxed");
        assert!(m.get("poolSize").is_none());
        assert_eq!(MASTER_MCP_TOOL_NAMES.len(), 14);
        assert!(MASTER_MCP_TOOL_NAMES.contains(&"observation_replay"));
        assert!(MASTER_MCP_TOOL_NAMES.contains(&"promote_to_apprentice_draft"));
    }

    #[test]
    fn render_schedule_prompt_placeholders() {
        let t =
            "执行 skill master-daily-digest，学徒={{apprentice_ids}}，窗口={{bizdate_yesterday}}";
        let out = render_schedule_prompt(t, &[10, 11], "20260802");
        assert!(out.contains("10,11"));
        assert!(out.contains("20260802"));
        assert!(!out.contains("{{"));
        let fallback = render_schedule_prompt("  ", &[1], "20260101");
        assert!(fallback.contains("master-daily-digest"));
        assert!(fallback.contains("1"));
    }

    #[test]
    fn master_mcp_shared_token_reads_env() {
        let _guard = test_env_lock();
        let prev = std::env::var("CLAW_MASTER_MCP_TOKEN").ok();
        std::env::set_var("CLAW_MASTER_MCP_TOKEN", "secret-x");
        assert_eq!(master_mcp_shared_token().as_deref(), Some("secret-x"));
        std::env::set_var("CLAW_MASTER_MCP_TOKEN", "  ");
        assert!(master_mcp_shared_token().is_none());
        match prev {
            Some(v) => std::env::set_var("CLAW_MASTER_MCP_TOKEN", v),
            None => std::env::remove_var("CLAW_MASTER_MCP_TOKEN"),
        }
    }

    #[test]
    fn new_ids_have_prefixes() {
        assert!(new_repair_run_id().starts_with("mrr_"));
        assert!(new_scheduled_job_id().starts_with("gsj_"));
    }
}
