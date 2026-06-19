//! Build tar.gz payloads for sandbox `guest_extract_tar_b64` (batch materialize). Author: kejiqing

use std::path::{Component, Path};

use base64::Engine;
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::Builder;

/// Validate tar member path (relative, no `..`). Author: kejiqing
pub fn validate_tar_member_path(rel: &str) -> Result<(), String> {
    let trimmed = rel.trim();
    if trimmed.is_empty() {
        return Err("empty tar member path".into());
    }
    if trimmed.starts_with('/') {
        return Err(format!("absolute tar member path: {trimmed}"));
    }
    let path = Path::new(trimmed);
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                return Err(format!("tar member path escapes with ..: {trimmed}"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("invalid tar member path: {trimmed}"));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    Ok(())
}

/// In-memory tar.gz → base64 for one `guest_extract_tar_b64` RPC. Author: kejiqing
pub fn build_tar_gz_b64(
    entries: &[(String, Vec<u8>)],
    max_total_bytes: usize,
) -> Result<String, String> {
    if entries.is_empty() {
        return Err("tar entries empty".into());
    }
    let mut total = 0usize;
    for (rel, bytes) in entries {
        validate_tar_member_path(rel)?;
        if bytes.len() > max_total_bytes {
            return Err(format!(
                "tar member {rel} exceeds cap {max_total_bytes} bytes"
            ));
        }
        total = total.saturating_add(bytes.len());
        if total > max_total_bytes {
            return Err(format!(
                "tar payload total {total} bytes exceeds cap {max_total_bytes}"
            ));
        }
    }

    let enc = GzEncoder::new(Vec::new(), Compression::fast());
    let mut tar = Builder::new(enc);
    for (rel, bytes) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(
            u64::try_from(bytes.len()).map_err(|_| format!("tar member too large: {rel}"))?,
        );
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, rel, &bytes[..])
            .map_err(|e| format!("tar append {rel}: {e}"))?;
    }
    tar.finish().map_err(|e| format!("tar finish: {e}"))?;
    let compressed = tar
        .into_inner()
        .map_err(|e| format!("tar into_inner: {e}"))?
        .finish()
        .map_err(|e| format!("gzip finish: {e}"))?;

    Ok(base64::engine::general_purpose::STANDARD.encode(compressed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_tar_gz_roundtrip_shape() {
        let entries = vec![
            (".claw/settings.json".to_string(), br#"{"k":1}"#.to_vec()),
            ("home/CLAUDE.md".to_string(), b"# hi".to_vec()),
        ];
        let b64 = build_tar_gz_b64(&entries, 1024 * 1024).expect("tar");
        assert!(!b64.is_empty());
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("b64");
        assert!(raw.starts_with(&[0x1f, 0x8b]));
    }

    #[test]
    fn rejects_parent_dir() {
        let entries = vec![("../etc/passwd".to_string(), vec![1])];
        let err = build_tar_gz_b64(&entries, 1024).expect_err("..");
        assert!(err.contains(".."));
    }
}
