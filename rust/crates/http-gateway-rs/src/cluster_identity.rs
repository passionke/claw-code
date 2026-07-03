//! PostgreSQL cluster identity + clawTap health contract. Author: kejiqing

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PgUrlParts {
    pub scheme: String,
    pub user: String,
    pub host: String,
    pub port: u16,
    pub dbname: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterIdentity {
    #[serde(rename = "clusterId")]
    pub cluster_id: String,
    #[serde(rename = "dbHost")]
    pub db_host: String,
    #[serde(rename = "clusterHash")]
    pub cluster_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TapHealthClusterFields {
    #[serde(default)]
    pub ok: bool,
    #[serde(default, rename = "clusterId")]
    pub cluster_id: Option<String>,
    #[serde(default, rename = "dbHost")]
    pub db_host: Option<String>,
    #[serde(default, rename = "clusterHash")]
    pub cluster_hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClusterMismatchError {
    pub message: String,
    pub cluster_match: bool,
    pub hash_match: bool,
}

/// Deploy cluster label (`.env` only; not derived from PG URL). Author: kejiqing
pub const CLUSTER_ID_ENV: &str = "CLAW_CLUSTER_ID";

pub fn validate_cluster_id(cluster_id: &str) -> Result<(), String> {
    let cluster_id = cluster_id.trim();
    if cluster_id.is_empty() || cluster_id.len() > 64 {
        return Err(format!("{CLUSTER_ID_ENV} is required (max 64 chars)"));
    }
    if !cluster_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "{CLUSTER_ID_ENV} must be alphanumeric, dash, or underscore"
        ));
    }
    Ok(())
}

/// `CLAW_CLUSTER_ID` from process environment. Author: kejiqing
pub fn gateway_cluster_id() -> Result<String, String> {
    let raw = std::env::var(CLUSTER_ID_ENV)
        .map_err(|_| format!("{CLUSTER_ID_ENV} is not set in deploy .env"))?;
    let cluster_id = raw.trim();
    validate_cluster_id(cluster_id)?;
    Ok(cluster_id.to_string())
}

pub fn gateway_cluster_id_optional() -> Option<String> {
    gateway_cluster_id().ok()
}

/// Parse `postgres://user:pass@host:port/dbname` (password ignored). Author: kejiqing
pub fn parse_pg_url(url: &str) -> Result<PgUrlParts, String> {
    let trimmed = url.trim();
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        return Err("database URL must include scheme".into());
    };
    if scheme != "postgres" && scheme != "postgresql" {
        return Err(format!("unsupported database scheme: {scheme}"));
    }
    let (auth_host, dbname) = rest
        .split_once('/')
        .map(|(a, b)| (a, b.split('?').next().unwrap_or(b)))
        .unwrap_or((rest, ""));
    let dbname = dbname.trim();
    if dbname.is_empty() {
        return Err("database URL missing dbname".into());
    }
    let (user, host_port) = if let Some((u, hp)) = auth_host.rsplit_once('@') {
        let user = u.split(':').next().unwrap_or(u).trim();
        if user.is_empty() {
            return Err("database URL missing user".into());
        }
        (user.to_string(), hp)
    } else {
        return Err("database URL missing user@host".into());
    };
    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        let port = p
            .parse::<u16>()
            .map_err(|_| format!("invalid port in database URL: {p}"))?;
        (h.to_string(), port)
    } else {
        (host_port.to_string(), 5432)
    };
    if host.trim().is_empty() {
        return Err("database URL missing host".into());
    }
    Ok(PgUrlParts {
        scheme: scheme.to_string(),
        user,
        host,
        port,
        dbname: dbname.to_string(),
    })
}

/// `SHA256(clusterId|scheme|user|dbname)` — clusterId + DB identity (scheme,user) + db name only; no host/port. Author: kejiqing
#[must_use]
pub fn compute_cluster_hash(cluster_id: &str, parts: &PgUrlParts) -> String {
    let payload = format!(
        "{}|{}|{}|{}",
        cluster_id.trim(),
        parts.scheme,
        parts.user,
        parts.dbname
    );
    let digest = Sha256::digest(payload.as_bytes());
    format!("sha256:{}", hex::encode(digest))
}

pub fn local_cluster_identity(
    cluster_id: &str,
    database_url: &str,
) -> Result<ClusterIdentity, String> {
    let cluster_id = cluster_id.trim();
    if cluster_id.is_empty() {
        return Err("clusterId is required".into());
    }
    let parts = parse_pg_url(database_url)?;
    Ok(ClusterIdentity {
        cluster_id: cluster_id.to_string(),
        db_host: parts.host.clone(),
        cluster_hash: compute_cluster_hash(cluster_id, &parts),
    })
}

pub fn verify_tap_cluster(
    local: &ClusterIdentity,
    tap: &ClusterIdentity,
) -> Result<(), ClusterMismatchError> {
    let cluster_match = local.cluster_id == tap.cluster_id;
    let hash_match = local.cluster_hash == tap.cluster_hash;
    if cluster_match && hash_match {
        return Ok(());
    }
    let mut parts = Vec::new();
    if !cluster_match {
        parts.push(format!(
            "clusterId mismatch (expected {}, tap reported {})",
            local.cluster_id, tap.cluster_id
        ));
    }
    if !hash_match {
        parts.push(format!(
            "clusterHash mismatch (expected {}, tap reported {})",
            local.cluster_hash, tap.cluster_hash
        ));
    }
    Err(ClusterMismatchError {
        message: parts.join("; "),
        cluster_match,
        hash_match,
    })
}

pub fn tap_identity_from_health(
    cluster_id: &str,
    fields: &TapHealthClusterFields,
) -> Result<ClusterIdentity, String> {
    let tap_cluster = fields
        .cluster_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "clawTap health missing clusterId".to_string())?;
    // dbHost is optional on the wire (claude-tap does not expose PG host publicly).
    let db_host = fields
        .db_host
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();
    let cluster_hash = fields
        .cluster_hash
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "clawTap health missing clusterHash".to_string())?;
    if tap_cluster != cluster_id.trim() {
        return Err(format!(
            "clawTap clusterId {tap_cluster} does not match configured {cluster_id}"
        ));
    }
    Ok(ClusterIdentity {
        cluster_id: tap_cluster.to_string(),
        db_host: db_host.to_string(),
        cluster_hash: cluster_hash.to_string(),
    })
}

pub fn gateway_database_url() -> Result<String, String> {
    std::env::var("CLAW_GATEWAY_DATABASE_URL")
        .map(|v| v.trim().to_string())
        .map_err(|_| "CLAW_GATEWAY_DATABASE_URL is not set".into())
        .and_then(|v| {
            if v.is_empty() {
                Err("CLAW_GATEWAY_DATABASE_URL is empty".into())
            } else {
                Ok(v)
            }
        })
}

fn worker_database_url() -> Result<String, String> {
    if let Ok(v) = std::env::var("CLAW_E2B_WORKER_DATABASE_URL") {
        let t = v.trim();
        if !t.is_empty() {
            return Ok(t.to_string());
        }
    }
    gateway_database_url()
}

fn replace_pg_url_host(url: &str, new_host: &str, new_port: u16) -> Result<String, String> {
    let trimmed = url.trim();
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        return Err("database URL must include scheme".into());
    };
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (auth, hostpath) = rest
        .rsplit_once('@')
        .ok_or_else(|| "database URL missing user@host".to_string())?;
    let (_, dbpath) = hostpath
        .split_once('/')
        .ok_or_else(|| "database URL missing dbname".to_string())?;
    let mut out = format!("{scheme}://{auth}@{new_host}:{new_port}/{dbpath}");
    if !query.is_empty() {
        out.push('?');
        out.push_str(query);
    }
    Ok(out)
}

/// PG URL injected into e2b sandboxes (remote host cannot use gateway `127.0.0.1`). Author: kejiqing
pub fn sandbox_database_url() -> Result<String, String> {
    if let Ok(v) = std::env::var("CLAW_E2B_SANDBOX_DATABASE_URL") {
        let t = v.trim();
        if !t.is_empty() {
            return Ok(t.to_string());
        }
    }
    let url = worker_database_url()?;
    let parts = parse_pg_url(&url)?;
    let host_lower = parts.host.to_lowercase();
    if !matches!(host_lower.as_str(), "127.0.0.1" | "localhost" | "::1") {
        return Ok(url);
    }
    let sandbox_host = std::env::var("CLAW_E2B_SANDBOX_PG_HOST")
        .or_else(|_| std::env::var("CLAW_POOL_ADVERTISE_HOST"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "CLAW_E2B_WORKER_DATABASE_URL uses 127.0.0.1 but e2b sandboxes run on another host; \
             set CLAW_E2B_SANDBOX_DATABASE_URL or CLAW_E2B_SANDBOX_PG_HOST"
                .to_string()
        })?;
    replace_pg_url_host(&url, &sandbox_host, parts.port)
}

/// GET `{tap_base}/healthz` and parse cluster fields. Author: kejiqing
pub async fn fetch_tap_cluster_identity(
    tap_base_url: &str,
    expected_cluster_id: &str,
) -> Result<ClusterIdentity, String> {
    let base = tap_base_url.trim().trim_end_matches('/');
    let probe_url = format!("{base}/healthz");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let resp = client
        .get(&probe_url)
        .send()
        .await
        .map_err(|e| format!("clawTap health fetch failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "clawTap health HTTP {} from {}",
            resp.status().as_u16(),
            probe_url
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("clawTap health JSON: {e}"))?;
    let fields: TapHealthClusterFields = if body.get("clusterId").is_some() {
        serde_json::from_value(body.clone()).map_err(|e| format!("clawTap health shape: {e}"))?
    } else if let Some(nested) = body.get("clawTap").or_else(|| body.get("cluster")) {
        serde_json::from_value(nested.clone())
            .map_err(|e| format!("clawTap nested health shape: {e}"))?
    } else {
        return Err("clawTap health missing clusterId (upgrade claude-tap)".into());
    };
    if !fields.ok {
        return Err("clawTap health ok=false".into());
    }
    tap_identity_from_health(expected_cluster_id, &fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_hash_stable() {
        let url = "postgres://claw_gateway:secret@postgres:5432/claw_gateway";
        let parts = parse_pg_url(url).unwrap();
        assert_eq!(parts.host, "postgres");
        assert_eq!(parts.port, 5432);
        assert_eq!(parts.dbname, "claw_gateway");
        let h1 = compute_cluster_hash("prod-01", &parts);
        let h2 = compute_cluster_hash("prod-01", &parts);
        assert_eq!(h1, h2);
        assert!(h1.starts_with("sha256:"));
    }

    #[test]
    fn validate_cluster_id_format() {
        assert!(validate_cluster_id("prod-claw-01").is_ok());
        assert!(validate_cluster_id("").is_err());
        assert!(validate_cluster_id("bad id").is_err());
    }

    #[test]
    fn verify_mismatch_cluster_id() {
        let local = ClusterIdentity {
            cluster_id: "a".into(),
            db_host: "h".into(),
            cluster_hash: "sha256:1".into(),
        };
        let tap = ClusterIdentity {
            cluster_id: "b".into(),
            db_host: "h".into(),
            cluster_hash: "sha256:1".into(),
        };
        let err = verify_tap_cluster(&local, &tap).unwrap_err();
        assert!(!err.cluster_match);
    }
}
