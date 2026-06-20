//! FC sandbox worker-embedded claude-tap (`127.0.0.1:8080`). Author: kejiqing

use std::collections::BTreeMap;

use crate::claw_tap_cluster_state::{active_llm_upstream, SolveLlmRoute};
use crate::cluster_identity::{gateway_cluster_id, gateway_database_url};
use crate::gateway_global_settings;
use crate::session_db::GatewaySessionDb;
use base64::Engine;

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
mkdir -p /claw_host_root/.claw/tap-traces
printf '%s' '{upstream_b64}' | base64 -d > /claw_host_root/.claw/claw-tap-upstream.json
if curl -fsS --connect-timeout 2 "http://127.0.0.1:{tap_port}/healthz" >/dev/null 2>&1; then
  exit 0
fi
nohup env CLAW_CLUSTER_ID={cluster_id:?} CLAW_GATEWAY_DATABASE_URL={db_url:?} \
  "$TAP_BIN" \
  --tap-no-launch \
  --tap-host 127.0.0.1 \
  --tap-port {tap_port} \
  --tap-target {upstream_target:?} \
  --tap-upstream-config /claw_host_root/.claw/claw-tap-upstream.json \
  --tap-output-dir /claw_host_root/.claw/tap-traces \
  >/claw_host_root/.claw/tap.log 2>&1 &
for _i in $(seq 1 45); do
  if curl -fsS --connect-timeout 2 "http://127.0.0.1:{tap_port}/healthz" >/dev/null 2>&1; then
    exit 0
  fi
  sleep 1
done
echo "fc worker tap: /healthz timeout (see /claw_host_root/.claw/tap.log)" >&2
exit 1
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

/// Solve path: rewrite route metadata for worker-local tap (gateway still probes pool tap).
#[must_use]
pub fn fc_worker_solve_route(mut route: SolveLlmRoute) -> SolveLlmRoute {
    route.claw_tap_base_url = Some(FC_WORKER_TAP_PROXY_URL.to_string());
    route
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
    }
}
