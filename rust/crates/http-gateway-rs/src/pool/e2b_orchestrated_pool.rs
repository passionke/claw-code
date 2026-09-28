//! e2b cloud sandbox pool — solve uses per-project workers from [`E2bProjWorkerRegistry`].
//! Author: kejiqing

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use claw_e2b_sandbox_client::E2bSandboxClient;
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};
use tracing::warn;

use crate::master_observer::PROJECT_ROLE_SCOPE;
use crate::project_config_apply::{build_settings_json_from_row, enabled_mcp_servers};
use crate::project_scope::{
    build_scope_key, mcp_bind_is_initialized, mcp_bind_snapshot, mcp_bind_values_match,
    parse_scope_json, render_mcp_template, scope_bind_values,
};
use crate::session_db::GatewaySessionDb;

use super::e2b_proj_worker_registry::E2bProjWorkerRegistry;
use super::merge_stdout_hooks;
use super::result::parse_gateway_solve_exec_stdout;
use super::session_db_sync::{
    finalize_turn_after_readback, readback_turn_from_session_home,
    sync_turn_progress_from_session_home, SESSION_MANIFEST_MAX_BYTES,
};
use super::traits::{PoolOps, SlotLease, TaskOutcome};
use super::{LiveReportHub, NasLayoutBackend};

pub const E2B_POOL_ID: &str = "e2b-cloud";

struct E2bSlot {
    sandbox_id: String,
    session_segment: String,
    proj_id: i64,
    worker_slot_index: u32,
    /// Empty = singleton pool; non-empty = scope role key. Author: kejiqing
    scope_key: String,
}

/// Per-turn leases on shared per-project worker sandboxes. Author: kejiqing
pub struct E2bOrchestratedPool {
    client: Arc<E2bSandboxClient>,
    nas_layout: NasLayoutBackend,
    workers: Arc<E2bProjWorkerRegistry>,
    db: RwLock<Option<Arc<GatewaySessionDb>>>,
    slots: Mutex<HashMap<usize, E2bSlot>>,
    turn_slots: Mutex<HashMap<String, usize>>,
    next_slot: AtomicUsize,
    live_report_hub: Arc<LiveReportHub>,
}

impl E2bOrchestratedPool {
    #[must_use]
    pub fn new(
        client: Arc<E2bSandboxClient>,
        live_report_hub: Arc<LiveReportHub>,
        nas_layout: NasLayoutBackend,
        workers: Arc<E2bProjWorkerRegistry>,
    ) -> Self {
        Self {
            client,
            nas_layout,
            workers,
            db: RwLock::new(None),
            slots: Mutex::new(HashMap::new()),
            turn_slots: Mutex::new(HashMap::new()),
            next_slot: AtomicUsize::new(1),
            live_report_hub,
        }
    }

    #[must_use]
    pub fn pool_id(&self) -> &'static str {
        E2B_POOL_ID
    }

    pub async fn bind_session_db(&self, db: Arc<GatewaySessionDb>) {
        *self.db.write().await = Some(db);
    }

    async fn session_db(&self) -> Result<Arc<GatewaySessionDb>, String> {
        self.db
            .read()
            .await
            .clone()
            .ok_or_else(|| "fc pool: session db not bound".into())
    }

    fn alloc_slot_index(&self) -> usize {
        self.next_slot.fetch_add(1, Ordering::Relaxed)
    }

    /// Per-turn task JSON only; transcript SoT is NAS `gateway-solve-session.jsonl`. Author: kejiqing
    async fn load_solve_task_json(
        &self,
        db: &GatewaySessionDb,
        turn_id: &str,
    ) -> Result<String, String> {
        let task = db
            .get_solve_task_json(turn_id)
            .await
            .map_err(|e| format!("load solve_task_json: {e}"))?
            .ok_or_else(|| format!("missing solve_task_json for turn {turn_id}"))?;
        let task_json = serde_json::to_string(&task).map_err(|e| format!("serialize task: {e}"))?;
        if task_json.len() > SESSION_MANIFEST_MAX_BYTES {
            return Err(format!(
                "solve_task_json exceeds cap {SESSION_MANIFEST_MAX_BYTES} bytes"
            ));
        }
        Ok(task_json)
    }
}

/// Bind MCP once per scope worker; same rendered settings → workers + sessions (claw reads session).
/// Project `mcp_servers_json` stays templated (`${…}`). Author: kejiqing
pub async fn ensure_scope_mcp_bind(
    db: &GatewaySessionDb,
    nas_layout: &NasLayoutBackend,
    proj_id: i64,
    scope_key: &str,
    worker_id: &str,
    session_segment: &str,
    slot: i32,
    extra_session: Option<&Value>,
    mcp_servers_template: &Value,
) -> Result<(), String> {
    let scope_json = db
        .get_scope_json(proj_id)
        .await
        .map_err(|e| format!("get_scope_json: {e}"))?;
    let cfg = parse_scope_json(&scope_json)?;
    let values = scope_bind_values(&cfg.scope_keys, extra_session)?;

    let row = db
        .get_project_e2b_worker_scoped(proj_id, scope_key, slot)
        .await
        .map_err(|e| format!("get worker for mcp bind: {e}"))?
        .ok_or_else(|| format!("missing worker for mcp bind proj_{proj_id} scope={scope_key}"))?;

    let config_row = db
        .get_project_config(proj_id)
        .await
        .map_err(|e| format!("load project_config for scope settings: {e}"))?;

    let mcp_servers = if mcp_bind_is_initialized(&row.mcp_bind_json) {
        mcp_bind_values_match(&row.mcp_bind_json, &values)?;
        row.mcp_bind_json
            .get("mcpServers")
            .cloned()
            .unwrap_or_else(|| json!({}))
    } else {
        let rendered = render_mcp_template(mcp_servers_template, &values);
        let mcp_servers = Value::Object(enabled_mcp_servers(&rendered));
        let snapshot = mcp_bind_snapshot(&values, &mcp_servers);
        db.update_project_e2b_worker_mcp_bind(proj_id, scope_key, slot, &snapshot)
            .await
            .map_err(|e| format!("save mcp_bind_json: {e}"))?;
        mcp_servers
    };

    write_scope_settings_mcp(
        nas_layout,
        proj_id,
        worker_id,
        session_segment,
        config_row.as_ref(),
        &mcp_servers,
    )
    .await
}

/// One render result → `workers/{id}/.claw/settings.json` and `sessions/{seg}/.claw/settings.json`.
async fn write_scope_settings_mcp(
    nas_layout: &NasLayoutBackend,
    proj_id: i64,
    worker_id: &str,
    session_segment: &str,
    config_row: Option<&crate::session_db::ProjectConfigRow>,
    mcp_servers: &Value,
) -> Result<(), String> {
    let mut settings = config_row
        .map(build_settings_json_from_row)
        .unwrap_or_else(|| {
            json!({
                "mcpServers": serde_json::Map::new(),
                "auto_hidden_system_prompt": 1
            })
        });
    if let Some(obj) = settings.as_object_mut() {
        obj.insert("mcpServers".to_string(), mcp_servers.clone());
    }
    let bytes = serde_json::to_vec_pretty(&settings)
        .map_err(|e| format!("serialize scope settings.json: {e}"))?;

    nas_layout.ensure_worker_root(proj_id, worker_id).await?;
    nas_layout
        .put_worker_file(proj_id, worker_id, ".claw/settings.json", &bytes)
        .await?;
    nas_layout
        .put_session_claw_file(proj_id, session_segment, "settings.json", &bytes)
        .await
}

#[async_trait]
impl PoolOps for E2bOrchestratedPool {
    async fn acquire_slot(
        &self,
        _wait: Duration,
        session_id: String,
        proj_id: i64,
        turn_id: String,
    ) -> Result<SlotLease, String> {
        let db = self.session_db().await?;
        db.assert_session_can_acquire_for_turn(&session_id, proj_id, &turn_id)
            .await
            .map_err(|reason| format!("session acquire blocked: {reason}"))?;

        let session_segment = crate::session_merge::sessions_directory_segment(&session_id);
        let role = db
            .get_project_role(proj_id)
            .await
            .map_err(|e| format!("get_project_role: {e}"))?;

        let (handle, _worker_id, worker_slot_index, scope_key) = if role == PROJECT_ROLE_SCOPE {
            let scope_json = db
                .get_scope_json(proj_id)
                .await
                .map_err(|e| format!("get_scope_json: {e}"))?;
            let cfg = parse_scope_json(&scope_json)?;
            let task = db
                .get_solve_task_json(&turn_id)
                .await
                .map_err(|e| format!("load solve_task_json for scope: {e}"))?
                .ok_or_else(|| format!("missing solve_task_json for turn {turn_id}"))?;
            let extra_session = task.get("extraSession");
            let scope_key = build_scope_key(&cfg.scope_keys, extra_session)?;
            let (handle, worker_id, slot) = self
                .workers
                .acquire_for_scope_solve(proj_id, &scope_key)
                .await?;
            let mcp_template = db
                .get_project_config(proj_id)
                .await
                .map_err(|e| format!("load project_config for mcp bind: {e}"))?
                .map(|r| r.mcp_servers_json)
                .unwrap_or_else(|| json!({}));
            self.nas_layout
                .ensure_session_context(proj_id, &session_segment, &worker_id)
                .await?;
            ensure_scope_mcp_bind(
                db.as_ref(),
                &self.nas_layout,
                proj_id,
                &scope_key,
                &worker_id,
                &session_segment,
                crate::session_db::e2b_worker_slot_i32(slot),
                extra_session,
                &mcp_template,
            )
            .await?;
            (handle, worker_id, slot, scope_key)
        } else {
            let (handle, worker_id, slot) =
                self.workers.acquire_for_solve(proj_id, &session_id).await?;
            self.nas_layout
                .ensure_session_context(proj_id, &session_segment, &worker_id)
                .await?;
            (handle, worker_id, slot, String::new())
        };

        let slot_index = self.alloc_slot_index();
        let worker_name = format!("e2b:{}", handle.sandbox_id);
        let _ = db
            .assign_turn_pool_worker(&turn_id, E2B_POOL_ID, &worker_name, Some("1000:1000"))
            .await;

        self.slots.lock().await.insert(
            slot_index,
            E2bSlot {
                sandbox_id: handle.sandbox_id.clone(),
                session_segment: session_segment.clone(),
                proj_id,
                worker_slot_index,
                scope_key,
            },
        );
        self.turn_slots.lock().await.insert(turn_id, slot_index);
        Ok(SlotLease { slot_index })
    }

    async fn exec_solve(
        &self,
        slot: &SlotLease,
        task_rel_under_root: &str,
        claw_bin: &str,
        _request_id: Option<&str>,
        turn_id: &str,
        timeout_seconds: u64,
        worker_llm_env: Option<BTreeMap<String, String>>,
        on_stdout_line: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Result<TaskOutcome, String> {
        let db = self.session_db().await?;
        let sandbox_id = self
            .slots
            .lock()
            .await
            .get(&slot.slot_index)
            .map(|s| s.sandbox_id.clone())
            .ok_or_else(|| format!("fc slot {} not found", slot.slot_index))?;

        self.turn_slots
            .lock()
            .await
            .insert(turn_id.to_string(), slot.slot_index);

        // Land per-turn task on NAS via nas-api; guest shell only runs short --task-file.
        // Author: kejiqing
        let (session_segment, proj_id) = {
            let slots = self.slots.lock().await;
            let s = slots
                .get(&slot.slot_index)
                .ok_or_else(|| format!("fc slot {} not found", slot.slot_index))?;
            (s.session_segment.clone(), s.proj_id)
        };

        let task_json = self.load_solve_task_json(db.as_ref(), turn_id).await?;
        self.nas_layout
            .write_session_task_json(proj_id, &session_segment, task_json.as_bytes())
            .await
            .map_err(|e| format!("nas-api write gateway-solve-task.json: {e}"))?;

        let stdout_hook = merge_stdout_hooks(
            turn_id,
            Some(Arc::clone(&self.live_report_hub)),
            Some(Arc::clone(&db)),
            on_stdout_line,
        );
        let outcome = self
            .client
            .exec_gateway_solve_once(
                &sandbox_id,
                task_rel_under_root,
                claw_bin,
                worker_llm_env.unwrap_or_default(),
                claw_e2b_sandbox_client::GatewaySolveInputs {
                    session_segment: &session_segment,
                    timeout_seconds,
                },
                stdout_hook,
            )
            .await?;

        let task_outcome = TaskOutcome {
            exit_code: outcome.exit_code,
            stdout: outcome.stdout.clone(),
            stderr: outcome.stderr.clone(),
        };

        if task_outcome.exit_code == 0 {
            if let Ok(Some((session_id, proj_id))) = db.turn_session_scope(turn_id).await {
                let user_prompt = db
                    .get_turn_user_prompt(turn_id)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if let Err(e) = readback_turn_from_session_home(
                    db.as_ref(),
                    &self.nas_layout,
                    &session_id,
                    proj_id,
                    turn_id,
                    &user_prompt,
                )
                .await
                {
                    warn!(
                        target: "claw_gateway_e2b_pool",
                        turn_id = %turn_id,
                        error = %e,
                        "fc readback from session home failed"
                    );
                } else {
                    let parsed = parse_gateway_solve_exec_stdout(
                        &task_outcome.stdout,
                        task_outcome.exit_code,
                    );
                    let worker_report = parsed.output_json.as_ref().and_then(|j| {
                        crate::biz_advice_report::report_body_from_solve_output(
                            &parsed.output_text,
                            Some(j),
                        )
                        .ok()
                    });
                    // Delegated turns: reportPath (worker passthrough disk) is canonical.
                    // Undelegated turns fall back to worker final answer. Author: kejiqing
                    let disk_report = self
                        .nas_layout
                        .read_session_claw_utf8(
                            proj_id,
                            &crate::session_merge::sessions_directory_segment(&session_id),
                            &gateway_solve_turn::router_delegate_report_claw_file(turn_id),
                        )
                        .await
                        .ok()
                        .flatten()
                        .filter(|s| !s.trim().is_empty());
                    let report = disk_report.or(worker_report);
                    let mut output_json = parsed.output_json.clone();
                    if let (Some(ref body), Some(ref mut j)) = (&report, &mut output_json) {
                        if let Some(obj) = j.as_object_mut() {
                            obj.insert("message".to_string(), serde_json::json!(body));
                        }
                    }
                    let _ = finalize_turn_after_readback(
                        db.as_ref(),
                        turn_id,
                        parsed.claw_exit_code,
                        report.as_deref(),
                        output_json.as_ref(),
                    )
                    .await;
                }
            }
        }

        Ok(task_outcome)
    }

    async fn release_slot(&self, slot: SlotLease) -> Result<(), String> {
        let removed = self.slots.lock().await.remove(&slot.slot_index);
        self.turn_slots
            .lock()
            .await
            .retain(|_, idx| *idx != slot.slot_index);
        if let Some(e2b_slot) = removed {
            self.workers
                .release_slot(
                    e2b_slot.proj_id,
                    e2b_slot.worker_slot_index,
                    &e2b_slot.scope_key,
                )
                .await;
        }
        Ok(())
    }

    async fn force_kill_slot(&self, slot_index: usize) -> Result<(), String> {
        self.release_slot(SlotLease { slot_index }).await
    }

    async fn has_report_for_turn(&self, turn_id: &str) -> bool {
        self.live_report_hub.has_report_for_turn(turn_id)
    }

    async fn first_report_at_ms_for_turn(&self, turn_id: &str) -> Option<i64> {
        self.live_report_hub.first_report_at_ms_for_turn(turn_id)
    }

    /// Running poll: nas-api read `.claw/progress*` → PG (no sandbox PG creds). Author: kejiqing
    async fn sync_turn_progress_to_db(&self, turn_id: &str) -> Result<(), String> {
        if turn_id.is_empty() {
            return Ok(());
        }
        let db = self.session_db().await?;
        let Some((session_id, proj_id)) = db
            .turn_session_scope(turn_id)
            .await
            .map_err(|e| format!("turn_session_scope: {e}"))?
        else {
            return Ok(());
        };
        let session_segment = crate::session_merge::sessions_directory_segment(&session_id);
        match sync_turn_progress_from_session_home(
            db.as_ref(),
            &self.nas_layout,
            proj_id,
            &session_segment,
            turn_id,
        )
        .await
        {
            Ok(()) => Ok(()),
            Err(e) => {
                warn!(
                    target: "claw_gateway_e2b_pool",
                    turn_id = %turn_id,
                    session_id = %session_id,
                    proj_id,
                    error = %e,
                    "sync_turn_progress_to_db via nas-api failed"
                );
                Err(e)
            }
        }
    }
}
