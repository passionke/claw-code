//! Session/turn client origin (`extraSession._claw_client_origin` or HTTP header). Author: kejiqing

use serde_json::Value;

pub const CLAW_EXTRA_CLIENT_ORIGIN: &str = "_claw_client_origin";
pub const CLIENT_ORIGIN_GATEWAY_ADMIN: &str = "gateway-admin";
pub const CLIENT_ORIGIN_OVS_CHAT: &str = "ovs-chat";
pub const HEADER_CLIENT_ORIGIN: &str = "x-claw-client-origin";

/// Resolve origin from HTTP header (preferred) or `extraSession._claw_client_origin`.
#[must_use]
pub fn resolve_client_origin(
    extra_session: Option<&Value>,
    header: Option<&str>,
) -> Option<String> {
    if let Some(h) = header.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(norm) = normalize_client_origin(h) {
            return Some(norm);
        }
    }
    let Value::Object(map) = extra_session? else {
        return None;
    };
    let Value::String(v) = map.get(CLAW_EXTRA_CLIENT_ORIGIN)? else {
        return None;
    };
    normalize_client_origin(v)
}

fn normalize_client_origin(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 64 {
        return None;
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        Some(s.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_from_extra_session() {
        let extra = json!({ "_claw_client_origin": "gateway-admin", "store_id": "S1" });
        assert_eq!(
            resolve_client_origin(Some(&extra), None).as_deref(),
            Some(CLIENT_ORIGIN_GATEWAY_ADMIN)
        );
    }

    #[test]
    fn header_overrides_extra_session() {
        let extra = json!({ "_claw_client_origin": "store-app" });
        assert_eq!(
            resolve_client_origin(Some(&extra), Some("gateway-admin")).as_deref(),
            Some(CLIENT_ORIGIN_GATEWAY_ADMIN)
        );
    }
}
