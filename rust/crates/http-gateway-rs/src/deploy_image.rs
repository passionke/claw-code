//! Deploy image ref/tag for `/healthz` (from `CLAW_GATEWAY_IMAGE_REF` / compose `GATEWAY_IMAGE`). Author: kejiqing

/// Compose `GATEWAY_IMAGE` value passed into the running gateway container.
pub fn image_ref_from_env() -> String {
    std::env::var("CLAW_GATEWAY_IMAGE_REF").unwrap_or_default()
}

/// User-facing tag: `local`, `release-v1.2.3`, or the image tag segment.
pub fn deploy_image_tag(image_ref: &str) -> String {
    let s = image_ref.trim();
    if s.is_empty() {
        return "unknown".to_string();
    }
    if let Some((_, digest)) = s.split_once('@') {
        let d = digest.trim();
        if d.len() >= 12 {
            return format!("digest:{}", &d[..12]);
        }
        return "digest".to_string();
    }
    let tag = s.rsplit(':').next().unwrap_or(s);
    if tag == "local" {
        return "local".to_string();
    }
    if tag.starts_with("release-") {
        return tag.to_string();
    }
    tag.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_tag() {
        assert_eq!(deploy_image_tag("claw-gateway-rs:local"), "local");
    }

    #[test]
    fn release_tag() {
        assert_eq!(
            deploy_image_tag("ghcr.io/acme/claw-code:release-v1.2.3"),
            "release-v1.2.3"
        );
    }

    #[test]
    fn empty_unknown() {
        assert_eq!(deploy_image_tag(""), "unknown");
    }
}
