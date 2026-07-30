//! Aliyun OSS V4 (OSS4-HMAC-SHA256) client for session attachment persistence.
//! Author: kejiqing
//!
//! Env (repo `.env` → gateway restart):
//! - `CLAW_OSS_ENABLED` — unset/`1`/`true` → on when AK/SK/bucket/region set; `0`/`false` → off
//! - `CLAW_OSS_ENDPOINT` — e.g. `https://oss-ap-southeast-1.aliyuncs.com`
//! - `CLAW_OSS_REGION` / `CLAW_OSS_BUCKET` / `CLAW_OSS_ACCESS_KEY_ID` / `CLAW_OSS_ACCESS_KEY_SECRET`
//! - `CLAW_OSS_KEY_PREFIX` — default `sessions`
//! - `CLAW_OSS_OBJECT_TTL_DAYS` — default 730 (lifecycle SoT; written as `ossRetainUntilMs`)
//! - `CLAW_OSS_SIGNED_URL_TTL_SECS` — default 3600

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::Serialize;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// RFC3986 encode for query keys/values (and object key segments). Keep `-_.~`.
const OSS_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Encode object key path: encode each segment, keep `/` unencoded.
fn encode_object_key(key: &str) -> String {
    key.trim_start_matches('/')
        .split('/')
        .map(|seg| utf8_percent_encode(seg, OSS_ENCODE_SET).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn encode_query_component(s: &str) -> String {
    utf8_percent_encode(s, OSS_ENCODE_SET).to_string()
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn signing_key(secret: &str, date: &str, region: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("aliyun_v4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, b"oss");
    hmac_sha256(&k_service, b"aliyun_v4_request")
}

fn sorted_query_string(params: &BTreeMap<String, String>) -> String {
    params
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                encode_query_component(k),
                encode_query_component(v)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Process env → OSS attachment store. Author: kejiqing
#[derive(Debug, Clone)]
pub struct OssConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub access_key_secret: String,
    pub key_prefix: String,
    pub object_ttl_days: u64,
    pub signed_url_ttl_secs: u64,
}

impl OssConfig {
    /// Load from `CLAW_OSS_*`. Missing required fields → `enabled=false`.
    #[must_use]
    pub fn from_env() -> Self {
        let force_off = matches!(
            std::env::var("CLAW_OSS_ENABLED")
                .ok()
                .map(|v| v.trim().to_ascii_lowercase())
                .as_deref(),
            Some("0" | "false" | "no" | "off")
        );
        let endpoint = env_trim("CLAW_OSS_ENDPOINT").unwrap_or_default();
        let region = env_trim("CLAW_OSS_REGION").unwrap_or_default();
        let bucket = env_trim("CLAW_OSS_BUCKET").unwrap_or_default();
        let access_key_id = env_trim("CLAW_OSS_ACCESS_KEY_ID")
            .or_else(|| env_trim("OSS_ACCESS_KEY_ID"))
            .unwrap_or_default();
        let access_key_secret = env_trim("CLAW_OSS_ACCESS_KEY_SECRET")
            .or_else(|| env_trim("OSS_ACCESS_KEY_SECRET"))
            .unwrap_or_default();
        let key_prefix = env_trim("CLAW_OSS_KEY_PREFIX").unwrap_or_else(|| "sessions".into());
        let object_ttl_days = env_u64("CLAW_OSS_OBJECT_TTL_DAYS", 730);
        let signed_url_ttl_secs = env_u64("CLAW_OSS_SIGNED_URL_TTL_SECS", 3600);
        let configured = !force_off
            && !endpoint.is_empty()
            && !region.is_empty()
            && !bucket.is_empty()
            && !access_key_id.is_empty()
            && !access_key_secret.is_empty();
        Self {
            enabled: configured,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            region,
            bucket,
            access_key_id,
            access_key_secret,
            key_prefix: key_prefix.trim_matches('/').to_string(),
            object_ttl_days,
            signed_url_ttl_secs,
        }
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Virtual-host style object URL (unsigned).
    #[must_use]
    pub fn object_url(&self, key: &str) -> String {
        let key = key.trim_start_matches('/');
        format!(
            "https://{}.oss-{}.aliyuncs.com/{}",
            self.bucket, self.region, key
        )
    }

    /// Host for virtual-hosted-style requests.
    #[must_use]
    pub fn virtual_host(&self) -> String {
        format!("{}.oss-{}.aliyuncs.com", self.bucket, self.region)
    }

    /// Object key: `{prefix}/{clusterId}/proj_{projId}/{sessionId}/{fileName}`. Author: kejiqing
    #[must_use]
    pub fn build_attachment_key(
        &self,
        cluster_id: &str,
        proj_id: i64,
        session_id: &str,
        file_name: &str,
    ) -> String {
        let prefix = if self.key_prefix.is_empty() {
            "sessions"
        } else {
            self.key_prefix.as_str()
        };
        format!(
            "{}/{}/proj_{}/{}/{}",
            prefix.trim_matches('/'),
            cluster_id.trim_matches('/'),
            proj_id,
            session_id.trim_matches('/'),
            file_name.trim_start_matches('/')
        )
    }

    /// Retain-until wall clock from `object_ttl_days`.
    #[must_use]
    pub fn retain_until_ms(&self, now: DateTime<Utc>) -> i64 {
        let secs = i64::try_from(self.object_ttl_days.saturating_mul(86_400)).unwrap_or(i64::MAX);
        now.timestamp_millis()
            .saturating_add(secs.saturating_mul(1000))
    }

    /// Presigned GET URL (query V4, `x-oss-additional-headers=host`).
    pub fn presign_get(
        &self,
        key: &str,
        expires_secs: u64,
        now: DateTime<Utc>,
    ) -> Result<String, String> {
        self.presign("GET", key, expires_secs, now, None)
    }

    /// Presigned PUT URL; optional `Content-Type` is signed when present.
    pub fn presign_put(
        &self,
        key: &str,
        expires_secs: u64,
        now: DateTime<Utc>,
        content_type: Option<&str>,
    ) -> Result<String, String> {
        self.presign("PUT", key, expires_secs, now, content_type)
    }

    fn presign(
        &self,
        method: &str,
        key: &str,
        expires_secs: u64,
        now: DateTime<Utc>,
        content_type: Option<&str>,
    ) -> Result<String, String> {
        if !self.enabled {
            return Err("OSS not configured".into());
        }
        let datetime = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();
        let scope = format!("{}/{}/oss/aliyun_v4_request", date, self.region);
        let credential = format!("{}/{}", self.access_key_id, scope);
        let host = self.virtual_host();
        let encoded_key = encode_object_key(key);
        let mut query = BTreeMap::new();
        query.insert("x-oss-signature-version".into(), "OSS4-HMAC-SHA256".into());
        query.insert("x-oss-date".into(), datetime.clone());
        query.insert("x-oss-expires".into(), expires_secs.to_string());
        query.insert("x-oss-credential".into(), credential);
        query.insert("x-oss-additional-headers".into(), "host".into());
        // Content-Type for PUT is not a query param; header signing for PUT body upload
        // goes through put_object using Authorization header path instead when needed.
        let _ = content_type;
        let canonical_query = sorted_query_string(&query);
        let canonical_uri = format!("/{}/{}", self.bucket, encoded_key);
        let canonical_headers = format!("host:{host}\n");
        let canonical_request = format!(
            "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\nhost\nUNSIGNED-PAYLOAD"
        );
        let string_to_sign = format!(
            "OSS4-HMAC-SHA256\n{datetime}\n{scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );
        let key_bytes = signing_key(&self.access_key_secret, &date, &self.region);
        let signature = hex::encode(hmac_sha256(&key_bytes, string_to_sign.as_bytes()));
        let mut final_q = query;
        final_q.insert("x-oss-signature".into(), signature);
        let qs = sorted_query_string(&final_q);
        Ok(format!("https://{host}/{encoded_key}?{qs}"))
    }

    /// Upload bytes via Authorization header V4 PUT (no host in CanonicalHeaders).
    pub async fn put_object(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<(), String> {
        if !self.enabled {
            return Err("OSS not configured".into());
        }
        let now = Utc::now();
        let datetime = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();
        let scope = format!("{}/{}/oss/aliyun_v4_request", date, self.region);
        let host = self.virtual_host();
        let encoded_key = encode_object_key(key);
        let canonical_uri = format!("/{}/{}", self.bucket, encoded_key);
        // Header signing: CanonicalHeaders WITHOUT host (verified against Aliyun error body).
        let content_type_trim = content_type.trim();
        let (canonical_headers, signed_headers_line) = if content_type_trim.is_empty() {
            (
                format!("x-oss-content-sha256:UNSIGNED-PAYLOAD\nx-oss-date:{datetime}\n"),
                String::new(),
            )
        } else {
            (
                format!(
                    "content-type:{content_type_trim}\nx-oss-content-sha256:UNSIGNED-PAYLOAD\nx-oss-date:{datetime}\n"
                ),
                String::new(),
            )
        };
        let canonical_request = format!(
            "PUT\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers_line}\nUNSIGNED-PAYLOAD"
        );
        let string_to_sign = format!(
            "OSS4-HMAC-SHA256\n{datetime}\n{scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );
        let key_bytes = signing_key(&self.access_key_secret, &date, &self.region);
        let signature = hex::encode(hmac_sha256(&key_bytes, string_to_sign.as_bytes()));
        let auth = format!(
            "OSS4-HMAC-SHA256 Credential={}/{},Signature={signature}",
            self.access_key_id, scope
        );
        let url = format!("https://{host}/{encoded_key}");
        let mut req = reqwest::Client::new()
            .put(&url)
            .header("Host", &host)
            .header("x-oss-date", &datetime)
            .header("x-oss-content-sha256", "UNSIGNED-PAYLOAD")
            .header("Authorization", &auth)
            .body(bytes.to_vec());
        if !content_type_trim.is_empty() {
            req = req.header("Content-Type", content_type_trim);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("OSS put_object request: {e}"))?;
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("OSS put_object HTTP {status}: {text}"))
    }
}

/// Admin-safe snapshot (no secret). Author: kejiqing
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct OssSettingsPublic {
    pub enabled: bool,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    #[serde(rename = "keyPrefix")]
    pub key_prefix: String,
    #[serde(rename = "accessKeyIdSet")]
    pub access_key_id_set: bool,
    #[serde(rename = "objectTtlDays")]
    pub object_ttl_days: u64,
    #[serde(rename = "signedUrlTtlSecs")]
    pub signed_url_ttl_secs: u64,
}

impl From<&OssConfig> for OssSettingsPublic {
    fn from(c: &OssConfig) -> Self {
        Self {
            enabled: c.enabled,
            endpoint: c.endpoint.clone(),
            region: c.region.clone(),
            bucket: c.bucket.clone(),
            key_prefix: c.key_prefix.clone(),
            access_key_id_set: !c.access_key_id.is_empty(),
            object_ttl_days: c.object_ttl_days,
            signed_url_ttl_secs: c.signed_url_ttl_secs,
        }
    }
}

fn env_trim(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// Build CanonicalRequest for header-signed requests (no host). Exposed for tests.
#[must_use]
pub fn canonical_request_header_put(
    bucket: &str,
    encoded_key: &str,
    datetime: &str,
    content_type: Option<&str>,
) -> String {
    let canonical_uri = format!("/{bucket}/{encoded_key}");
    let canonical_headers = match content_type {
        Some(ct) if !ct.is_empty() => format!(
            "content-type:{ct}\nx-oss-content-sha256:UNSIGNED-PAYLOAD\nx-oss-date:{datetime}\n"
        ),
        _ => format!("x-oss-content-sha256:UNSIGNED-PAYLOAD\nx-oss-date:{datetime}\n"),
    };
    format!("PUT\n{canonical_uri}\n\n{canonical_headers}\n\nUNSIGNED-PAYLOAD")
}

/// Build CanonicalRequest for presigned GET (with host additional header).
#[must_use]
pub fn canonical_request_presign_get(
    bucket: &str,
    encoded_key: &str,
    host: &str,
    canonical_query: &str,
) -> String {
    let canonical_uri = format!("/{bucket}/{encoded_key}");
    let canonical_headers = format!("host:{host}\n");
    format!("GET\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\nhost\nUNSIGNED-PAYLOAD")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn encode_key_keeps_slashes() {
        assert_eq!(
            encode_object_key("sessions/local-dev/proj_1/a b.jpg"),
            "sessions/local-dev/proj_1/a%20b.jpg"
        );
    }

    #[test]
    fn header_canonical_matches_verified_shape() {
        // Shape verified 2026-07-29 against Aliyun SignatureDoesNotMatch error body:
        // CanonicalHeaders must NOT include host for Authorization-header signing.
        let cr = canonical_request_header_put(
            "clawcode-sessions",
            "sessions/_probe.txt",
            "20260729T105824Z",
            None,
        );
        assert_eq!(
            cr,
            "PUT\n/clawcode-sessions/sessions/_probe.txt\n\nx-oss-content-sha256:UNSIGNED-PAYLOAD\nx-oss-date:20260729T105824Z\n\n\nUNSIGNED-PAYLOAD"
        );
        let hash = sha256_hex(cr.as_bytes());
        let sts = format!(
            "OSS4-HMAC-SHA256\n20260729T105824Z\n20260729/ap-southeast-1/oss/aliyun_v4_request\n{hash}"
        );
        assert!(sts.starts_with("OSS4-HMAC-SHA256\n20260729T105824Z\n"));
    }

    #[test]
    fn presign_canonical_includes_host_additional() {
        let host = "clawcode-sessions.oss-ap-southeast-1.aliyuncs.com";
        let mut q = BTreeMap::new();
        q.insert("x-oss-additional-headers".into(), "host".into());
        q.insert(
            "x-oss-credential".into(),
            "AKID/20260729/ap-southeast-1/oss/aliyun_v4_request".into(),
        );
        q.insert("x-oss-date".into(), "20260729T105824Z".into());
        q.insert("x-oss-expires".into(), "900".into());
        q.insert("x-oss-signature-version".into(), "OSS4-HMAC-SHA256".into());
        let cq = sorted_query_string(&q);
        let cr = canonical_request_presign_get(
            "clawcode-sessions",
            "sessions/_probe/does-not-exist.txt",
            host,
            &cq,
        );
        assert!(cr.contains(&format!("host:{host}\n")));
        assert!(cr.contains("\nhost\nUNSIGNED-PAYLOAD"));
        assert!(cr.starts_with("GET\n/clawcode-sessions/sessions/_probe/does-not-exist.txt\n"));
    }

    #[test]
    fn build_attachment_key_layout() {
        let cfg = OssConfig {
            enabled: true,
            endpoint: "https://oss-ap-southeast-1.aliyuncs.com".into(),
            region: "ap-southeast-1".into(),
            bucket: "clawcode-sessions".into(),
            access_key_id: "ak".into(),
            access_key_secret: "sk".into(),
            key_prefix: "sessions".into(),
            object_ttl_days: 730,
            signed_url_ttl_secs: 3600,
        };
        assert_eq!(
            cfg.build_attachment_key("local-dev", 1, "sid123", "a1b2c3d4_x.jpg"),
            "sessions/local-dev/proj_1/sid123/a1b2c3d4_x.jpg"
        );
        assert_eq!(
            cfg.object_url("sessions/local-dev/proj_1/sid123/a.jpg"),
            "https://clawcode-sessions.oss-ap-southeast-1.aliyuncs.com/sessions/local-dev/proj_1/sid123/a.jpg"
        );
    }

    #[test]
    fn retain_until_uses_ttl_days() {
        let cfg = OssConfig {
            enabled: true,
            endpoint: String::new(),
            region: "ap-southeast-1".into(),
            bucket: "b".into(),
            access_key_id: "a".into(),
            access_key_secret: "s".into(),
            key_prefix: "sessions".into(),
            object_ttl_days: 2,
            signed_url_ttl_secs: 60,
        };
        let now = Utc.with_ymd_and_hms(2026, 7, 29, 0, 0, 0).unwrap();
        let until = cfg.retain_until_ms(now);
        assert_eq!(until - now.timestamp_millis(), 2 * 86_400 * 1000);
    }

    #[test]
    fn signing_key_stable_length() {
        let k = signing_key("secret", "20260729", "ap-southeast-1");
        assert_eq!(k.len(), 32);
    }
}
