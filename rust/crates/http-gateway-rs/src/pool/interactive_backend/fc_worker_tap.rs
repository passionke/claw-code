//! FC worker co-located claude-tap **proxy** (`127.0.0.1:8080`): LLM 代理 + session trace **写入**。
//! Worker 内 tap：proxy + trace 写入 NAS；Live 观察由 observe-singleton 模板 startCmd 负责（gateway.sh observe-tap-up）。
//! Author: kejiqing

use std::collections::BTreeMap;

use crate::claw_tap_cluster_state::{active_llm_upstream, SolveLlmRoute};
use crate::cluster_identity::{gateway_cluster_id, gateway_database_url, local_cluster_identity};
use crate::gateway_global_settings;
use crate::session_db::GatewaySessionDb;
use base64::Engine;
use claw_fc_sandbox_client::GUEST_CLAW_TAP_TRACES;

pub const FC_WORKER_TAP_PROXY_URL: &str = "http://127.0.0.1:8080";
const FC_WORKER_TAP_PORT: u16 = 8080;

/// Worker LLM env must hit co-located tap, not compose `claw-claude-tap`.
#[must_use]
pub fn fc_worker_llm_env(mut env: BTreeMap<String, String>) -> BTreeMap<String, String> {
    env.insert(
        "OPENAI_BASE_URL".to_string(),
        FC_WORKER_TAP_PROXY_URL.to_string(),
    );
    env.insert(
        "INTERNAL_CLAUDE_TAP_HOST".to_string(),
        FC_WORKER_TAP_PROXY_URL.to_string(),
    );
    env
}

/// PG URL reachable from FC sandbox (VPN). Falls back to gateway env when unset.
pub fn fc_worker_tap_database_url() -> Result<String, String> {
    if let Ok(raw) = std::env::var("CLAW_FC_WORKER_DATABASE_URL") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    gateway_database_url()
}

/// Idempotent shell: stage tap binary, write upstream config, start tap, wait `/healthz`.
#[must_use]
pub fn build_fc_worker_tap_start_script(
    cluster_id: &str,
    db_url: &str,
    upstream_target: &str,
) -> String {
    let upstream_json = serde_json::json!({ "target": upstream_target.trim() });
    let upstream_body = upstream_json.to_string();
    let upstream_b64 = base64::engine::general_purpose::STANDARD.encode(upstream_body.as_bytes());
    let tap_port = FC_WORKER_TAP_PORT;
    let tap_traces_dir = GUEST_CLAW_TAP_TRACES;
    format!(
        r#"set -e
TAP_BIN=""
for cand in /usr/local/bin/claude-tap /tmp/claw-fc-bin/claude-tap /tmp/claw-fc-bin/tap-runtime/bin/claude-tap; do
  if [ -x "$cand" ]; then TAP_BIN="$cand"; break; fi
done
if [ -z "$TAP_BIN" ]; then
  echo "fc worker tap: claude-tap not found (rebuild claw-worker template or install-nas-fc-tools)" >&2
  exit 127
fi
mkdir -p {tap_traces_dir}
printf '%s' '{upstream_b64}' | base64 -d > /claw_host_root/.claw/claw-tap-upstream.json
if ! curl -fsS --connect-timeout 2 "http://127.0.0.1:{tap_port}/healthz" >/dev/null 2>&1; then
  nohup env CLAW_CLUSTER_ID={cluster_id:?} CLAW_GATEWAY_DATABASE_URL={db_url:?} \
    "$TAP_BIN" \
    --tap-no-launch \
    --tap-host 127.0.0.1 \
    --tap-port {tap_port} \
    --tap-target {upstream_target:?} \
    --tap-upstream-config /claw_host_root/.claw/claw-tap-upstream.json \
    --tap-output-dir {tap_traces_dir} \
    >/claw_host_root/.claw/tap.log 2>&1 &
  for _i in $(seq 1 45); do
    if curl -fsS --connect-timeout 2 "http://127.0.0.1:{tap_port}/healthz" >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done
  if ! curl -fsS --connect-timeout 2 "http://127.0.0.1:{tap_port}/healthz" >/dev/null 2>&1; then
    echo "fc worker tap: /healthz timeout (see /claw_host_root/.claw/tap.log)" >&2
    exit 1
  fi
fi
"#
    )
}

pub async fn build_fc_worker_tap_start_script_from_db(
    session_db: &GatewaySessionDb,
) -> Result<String, String> {
    let cluster_id = gateway_cluster_id()?;
    let db_url = fc_worker_tap_database_url()?;
    let active = gateway_global_settings::load_active_llm_runtime(session_db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no active LLM model configured in Admin".to_string())?;
    let (upstream, _) = active_llm_upstream(&active)?;
    Ok(build_fc_worker_tap_start_script(
        &cluster_id,
        &db_url,
        &upstream,
    ))
}

pub async fn build_fc_session_attach_with_tap(
    session_db: &GatewaySessionDb,
    llm_env: &BTreeMap<String, String>,
) -> Result<String, String> {
    let tap = build_fc_worker_tap_start_script_from_db(session_db).await?;
    let attach = super::build_session_attach_script(llm_env);
    Ok(format!("{tap}\n{attach}"))
}

/// Solve path: rewrite route metadata for worker-local tap (no compose `claw-claude-tap` probe).
#[must_use]
pub fn fc_worker_solve_route(mut route: SolveLlmRoute) -> SolveLlmRoute {
    route.claw_tap_base_url = Some(FC_WORKER_TAP_PROXY_URL.to_string());
    route
}

/// fc-cloud solve: PG active LLM + worker tap proxy (127.0.0.1:8080); skip compose clawTap /healthz.
pub async fn resolve_fc_worker_solve_llm_route(
    session_db: &GatewaySessionDb,
    model_override: Option<&str>,
) -> Result<(SolveLlmRoute, BTreeMap<String, String>), String> {
    let cluster_id = gateway_cluster_id()?;
    let db_url = gateway_database_url()?;
    let local = local_cluster_identity(&cluster_id, &db_url)?;
    let active = gateway_global_settings::load_active_llm_runtime(session_db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no active LLM model configured in Admin".to_string())?;
    let (upstream, default_model) = active_llm_upstream(&active)?;
    let model = model_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or(default_model);
    let api_key = active.api_key.trim();
    if api_key.is_empty() {
        return Err("active LLM apiKey missing".into());
    }
    let route = fc_worker_solve_route(SolveLlmRoute {
        mode: "fcWorkerTap".to_string(),
        cluster_id: cluster_id.clone(),
        cluster_hash: local.cluster_hash.clone(),
        claw_tap_base_url: Some(FC_WORKER_TAP_PROXY_URL.to_string()),
        upstream_base_url: upstream,
        model: model.clone(),
        reason: None,
    });
    let mut env = BTreeMap::new();
    env.insert("OPENAI_API_KEY".to_string(), api_key.to_string());
    env.insert("CLAW_DEFAULT_MODEL".to_string(), model);
    Ok((route, fc_worker_llm_env(env)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fc_worker_llm_env_points_at_localhost_tap() {
        let mut env = BTreeMap::new();
        env.insert(
            "OPENAI_BASE_URL".to_string(),
            "http://claw-claude-tap:8080".to_string(),
        );
        let out = fc_worker_llm_env(env);
        assert_eq!(
            out.get("OPENAI_BASE_URL").map(String::as_str),
            Some(FC_WORKER_TAP_PROXY_URL)
        );
    }

    #[test]
    fn tap_start_script_mentions_local_healthz() {
        let sh = build_fc_worker_tap_start_script(
            "local-dev",
            "postgres://u:p@10.8.0.10:5433/claw_gateway",
            "https://example.com/v1",
        );
        assert!(sh.contains("127.0.0.1:8080"));
        assert!(sh.contains("claude-tap"));
        assert!(sh.contains("claw-tap-upstream.json"));
        assert!(sh.contains(GUEST_CLAW_TAP_TRACES));
        // Must not exit early when tap is already up — attach script continues to ttyd.
        assert!(!sh.contains("exit 0"));
    }
}
