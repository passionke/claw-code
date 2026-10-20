//! Admin: ensure fingerprint SKU e2b worker template + bind resourceTemplate.
//! Gateway spawns `deploy/e2b/ensure-worker-resource-template.py` (same pattern as
//! bootstrap publish-templates). CLI remains an escape hatch only.
//! Author: kejiqing

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::gateway_bootstrap_deploy::resolve_bootstrap_repo_root;
use crate::pool::{
    effective_mode, effective_resource_template, normalize_worker_resources,
    relaxed_worker_allowed_from_env, resource_template_alias, sanitize_worker_profile_json,
    WorkerProfileMode, WorkerResources,
};
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ResourceTemplateEnsurePhase {
    Idle,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceTemplateEnsureJob {
    pub phase: ResourceTemplateEnsurePhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proj_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_tail: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceTemplateEnsureResponse {
    pub accepted: bool,
    pub job: ResourceTemplateEnsureJob,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

struct EnsureState {
    running: bool,
    job: ResourceTemplateEnsureJob,
}

fn ensure_state() -> &'static Mutex<EnsureState> {
    static STATE: OnceLock<Mutex<EnsureState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(EnsureState {
            running: false,
            job: ResourceTemplateEnsureJob {
                phase: ResourceTemplateEnsurePhase::Idle,
                proj_id: None,
                fingerprint: None,
                started_at_ms: None,
                finished_at_ms: None,
                message: None,
                log_tail: Vec::new(),
            },
        })
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn push_log(job: &mut ResourceTemplateEnsureJob, line: String) {
    const MAX: usize = 200;
    job.log_tail.push(line);
    if job.log_tail.len() > MAX {
        let drop_n = job.log_tail.len() - MAX;
        job.log_tail.drain(0..drop_n);
    }
}

fn current_job() -> ResourceTemplateEnsureJob {
    ensure_state()
        .lock()
        .map(|g| g.job.clone())
        .unwrap_or_else(|_| ResourceTemplateEnsureJob {
            phase: ResourceTemplateEnsurePhase::Failed,
            proj_id: None,
            fingerprint: None,
            started_at_ms: None,
            finished_at_ms: None,
            message: Some("ensure state lock poisoned".into()),
            log_tail: Vec::new(),
        })
}

#[must_use]
pub fn get_ensure_resource_template_status() -> ResourceTemplateEnsureResponse {
    ResourceTemplateEnsureResponse {
        accepted: true,
        job: current_job(),
        message: None,
    }
}

fn resolve_ensure_script() -> Result<PathBuf, String> {
    // Prefer image-bundled script (release hosts may mount a stale CLAW_REPO_ROOT).
    // Same pattern as pool/agent_engine_inject.rs. Author: kejiqing
    let bundled = PathBuf::from("/app/deploy/e2b/ensure-worker-resource-template.py");
    if bundled.is_file() {
        return Ok(bundled);
    }
    let root = resolve_bootstrap_repo_root().ok_or_else(|| {
        "repo root missing (set CLAW_REPO_ROOT so Gateway can run ensure-worker-resource-template.py)"
            .to_string()
    })?;
    let script = root.join("deploy/e2b/ensure-worker-resource-template.py");
    if !script.is_file() {
        return Err(format!("missing {}", script.display()));
    }
    Ok(script)
}

fn resources_from_profile(wp: &Value) -> Result<WorkerResources, String> {
    let Some(res) = wp.get("resources") else {
        return Err(
            "project has no workerProfileJson.resources; save custom cpu/memory in Admin first"
                .into(),
        );
    };
    let cpu = res
        .get("cpuCount")
        .and_then(Value::as_f64)
        .ok_or_else(|| "resources.cpuCount required".to_string())?;
    let mem = res
        .get("memoryMb")
        .and_then(Value::as_f64)
        .ok_or_else(|| "resources.memoryMb required".to_string())?;
    normalize_worker_resources(cpu, mem)
}

fn parse_script_resource_template(stdout: &str) -> Result<Value, String> {
    for line in stdout.lines().rev() {
        let t = line.trim();
        if !(t.starts_with('{') && t.contains("resourceTemplate")) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(t) else {
            continue;
        };
        if let Some(rt) = v.get("resourceTemplate") {
            return Ok(rt.clone());
        }
    }
    Err("ensure script succeeded but stdout had no resourceTemplate JSON line".into())
}

async fn bind_resource_template(
    db: &GatewaySessionDb,
    proj_id: i64,
    resources: &WorkerResources,
    resource_template: Value,
) -> Result<(), String> {
    let mut wp = db
        .get_worker_profile_json(proj_id)
        .await
        .map_err(|e| e.to_string())?;
    if !wp.is_object() {
        wp = json!({"mode": "strict"});
    }
    let map = wp
        .as_object_mut()
        .ok_or_else(|| "worker_profile_json must be object".to_string())?;
    map.insert("resources".into(), resources.to_json());
    map.insert("resourceTemplate".into(), resource_template);
    let cleaned = sanitize_worker_profile_json(&wp)?;
    if effective_resource_template(&cleaned).is_none() {
        return Err("bound resourceTemplate fingerprint does not match resources".into());
    }
    sqlx::query(
        r"UPDATE project_config
           SET worker_profile_json = $3, updated_at_ms = $4
         WHERE cluster_id = $1 AND proj_id = $2",
    )
    .bind(db.cluster_id())
    .bind(proj_id)
    .bind(sqlx::types::Json(cleaned))
    .bind(now_ms())
    .execute(db.pg_pool())
    .await
    .map_err(|e| format!("update worker_profile_json: {e}"))?;
    Ok(())
}

async fn run_ensure_script(
    script: &Path,
    repo: &Path,
    cpu: u32,
    memory_mb: u32,
    relaxed: bool,
) -> Result<String, String> {
    let mut cmd = Command::new("python3");
    cmd.arg(script)
        .arg("--cpu")
        .arg(cpu.to_string())
        .arg("--memory-mb")
        .arg(memory_mb.to_string())
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if relaxed {
        cmd.arg("--relaxed");
    }
    // SKU path: e2b API + PREFIX/BASE_TAG only (script does Template.build, not docker).
    // Author: kejiqing
    for key in [
        "REGION",
        "region",
        "CLAW_REGION",
        "CLAW_E2B_CN",
        "CLAW_E2B_API_KEY",
        "E2B_API_KEY",
        "CLAW_E2B_API_URL",
        "E2B_API_URL",
        "CLAW_E2B_SANDBOX_URL",
        "E2B_SANDBOX_URL",
        "CLAW_E2B_DOMAIN",
        "E2B_DOMAIN",
        "CLAW_IMAGE_PREFIX",
        "CLAW_E2B_WORKER_BASE_TAG",
        "CLAW_E2B_TEMPLATE_SKIP_CACHE",
    ] {
        if let Ok(v) = std::env::var(key) {
            let t = v.trim();
            if !t.is_empty() {
                cmd.env(key, t);
            }
        }
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn ensure-worker-resource-template.py: {e}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let out_task = async {
        let mut buf = String::new();
        if let Some(out) = stdout {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut guard) = ensure_state().lock() {
                    push_log(&mut guard.job, line.clone());
                }
                buf.push_str(&line);
                buf.push('\n');
            }
        }
        buf
    };
    let err_task = async {
        let mut buf = String::new();
        if let Some(err) = stderr {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut guard) = ensure_state().lock() {
                    push_log(&mut guard.job, format!("[stderr] {line}"));
                }
                buf.push_str(&line);
                buf.push('\n');
            }
        }
        buf
    };
    let (out_s, err_s) = tokio::join!(out_task, err_task);

    let status = child
        .wait()
        .await
        .map_err(|e| format!("wait ensure script: {e}"))?;
    if !status.success() {
        let code = status.code().unwrap_or(-1);
        let tail: Vec<&str> = err_s.lines().rev().take(8).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!("ensure script exit {code}: {}", tail.join(" | ")));
    }
    Ok(out_s)
}

/// Start async ensure for project resources; bind resourceTemplate when build finishes.
pub async fn start_ensure_resource_template(
    db: Arc<GatewaySessionDb>,
    proj_id: i64,
) -> Result<ResourceTemplateEnsureResponse, String> {
    if proj_id <= 0 {
        return Err("projId must be > 0".into());
    }
    let wp = db
        .get_worker_profile_json(proj_id)
        .await
        .map_err(|e| e.to_string())?;
    let resources = resources_from_profile(&wp)?;
    let relaxed =
        effective_mode(relaxed_worker_allowed_from_env(), &wp) == WorkerProfileMode::Relaxed;
    let fp = resources.fingerprint();
    if let Some(existing) = effective_resource_template(&wp) {
        if existing.fingerprint == fp {
            return Ok(ResourceTemplateEnsureResponse {
                accepted: false,
                job: current_job(),
                message: Some(format!(
                    "already bound fingerprint={fp} templateId={}",
                    existing.template_id
                )),
            });
        }
    }

    let script = resolve_ensure_script()?;
    // cwd for dotenv / relative paths; bundled script does not require a host checkout.
    let repo = resolve_bootstrap_repo_root().unwrap_or_else(|| PathBuf::from("/app"));

    {
        let mut guard = ensure_state()
            .lock()
            .map_err(|_| "ensure state lock poisoned".to_string())?;
        if guard.running {
            return Ok(ResourceTemplateEnsureResponse {
                accepted: false,
                job: guard.job.clone(),
                message: Some("resource template ensure already running".into()),
            });
        }
        guard.running = true;
        guard.job = ResourceTemplateEnsureJob {
            phase: ResourceTemplateEnsurePhase::Running,
            proj_id: Some(proj_id),
            fingerprint: Some(fp.clone()),
            started_at_ms: Some(now_ms()),
            finished_at_ms: None,
            message: Some(format!(
                "ensuring {} (cpu={} memoryMb={})",
                resource_template_alias(&fp, relaxed),
                resources.cpu_count,
                resources.memory_mb
            )),
            log_tail: vec![format!("==> start proj={proj_id} fingerprint={fp}")],
        };
    }

    let cpu = resources.cpu_count;
    let memory_mb = resources.memory_mb;
    tokio::spawn(async move {
        let outcome = async {
            let stdout = run_ensure_script(&script, &repo, cpu, memory_mb, relaxed).await?;
            let rt = parse_script_resource_template(&stdout)?;
            bind_resource_template(&db, proj_id, &resources, rt).await?;
            Ok::<(), String>(())
        }
        .await;

        let Ok(mut guard) = ensure_state().lock() else {
            return;
        };
        guard.running = false;
        guard.job.finished_at_ms = Some(now_ms());
        match outcome {
            Ok(()) => {
                guard.job.phase = ResourceTemplateEnsurePhase::Succeeded;
                guard.job.message = Some(format!(
                    "bound proj={proj_id} fingerprint={fp}; force-reset workers to apply"
                ));
                push_log(&mut guard.job, "==> succeeded".into());
                info!(
                    target: "claw_gateway_resource_template",
                    proj_id,
                    fingerprint = %fp,
                    "resource template ensure succeeded"
                );
            }
            Err(err) => {
                guard.job.phase = ResourceTemplateEnsurePhase::Failed;
                guard.job.message = Some(err.clone());
                push_log(&mut guard.job, format!("==> failed: {err}"));
                warn!(
                    target: "claw_gateway_resource_template",
                    proj_id,
                    fingerprint = %fp,
                    error = %err,
                    "resource template ensure failed"
                );
            }
        }
    });

    Ok(ResourceTemplateEnsureResponse {
        accepted: true,
        job: current_job(),
        message: Some("ensure started".into()),
    })
}
