//! Reverse-proxy live SSE / cancel to the turn-owning gateway. Author: kejiqing
//!
//! When Admin or a client hits a non-owner gateway for a running turn, this module
//! proxies to `{owner.gateway_base}` so the upstream stays sticky-unaware.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;

use crate::gateway_endpoint::{is_gateway_endpoint_online, GatewayEndpointIdentity};
use crate::session_db::GatewaySessionDb;

/// Decide whether this process owns the turn; if not, return owner base URL when online.
pub async fn resolve_turn_owner_proxy_base(
    db: &GatewaySessionDb,
    self_identity: &GatewayEndpointIdentity,
    turn_id: &str,
    session_id: &str,
    proj_id: i64,
) -> Result<Option<String>, String> {
    let Some((owner_id, owner_base)) = db
        .get_turn_gateway_owner(turn_id, session_id, proj_id)
        .await
        .map_err(|e| e.to_string())?
    else {
        // Legacy turns without owner: treat as local (best-effort). Author: kejiqing
        return Ok(None);
    };
    if owner_id == self_identity.gateway_id {
        return Ok(None);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    if let Ok(Some(ep)) = db.get_gateway_endpoint(&owner_id).await {
        if is_gateway_endpoint_online(ep.last_heartbeat_ms, now) {
            return Ok(Some(ep.gateway_base));
        }
    }
    // Fall back to turn-recorded base even if heartbeat stale (may still work).
    if !owner_base.trim().is_empty() {
        return Ok(Some(owner_base.trim_end_matches('/').to_string()));
    }
    Err(format!(
        "turn {turn_id} owned by gateway {owner_id} which is offline"
    ))
}

/// Stream-proxy GET/POST to owning gateway; returns axum Response. Author: kejiqing
pub async fn proxy_to_owner_gateway(
    owner_base: &str,
    method: &str,
    path_and_query: &str,
    forward_headers: &HeaderMap,
) -> Result<Response, String> {
    let base = owner_base.trim().trim_end_matches('/');
    let url = format!("{base}{path_and_query}");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("proxy client: {e}"))?;
    let mut req = match method.to_uppercase().as_str() {
        "POST" => client.post(&url),
        "DELETE" => client.delete(&url),
        _ => client.get(&url),
    };
    // Forward Accept / Authorization when present; skip hop-by-hop. Author: kejiqing
    for name in ["accept", "authorization", "cookie"] {
        if let Some(v) = forward_headers.get(name) {
            if let Ok(s) = v.to_str() {
                req = req.header(name, s);
            }
        }
    }
    let upstream = req
        .send()
        .await
        .map_err(|e| format!("proxy to owner gateway failed: {e}"))?;
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for (k, v) in upstream.headers() {
        let name = k.as_str();
        if matches!(
            name,
            "transfer-encoding" | "connection" | "keep-alive" | "content-length"
        ) {
            continue;
        }
        if let (Ok(hn), Ok(hv)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_bytes(v.as_bytes()),
        ) {
            builder = builder.header(hn, hv);
        }
    }
    let stream = upstream
        .bytes_stream()
        .map(|chunk| chunk.map_err(|e| std::io::Error::other(format!("upstream sse chunk: {e}"))));
    builder
        .body(Body::from_stream(stream))
        .map_err(|e| format!("build proxy response: {e}"))
}
