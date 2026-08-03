//! Skill package (tar/tgz) encode/decode for `skills_json`. Author: kejiqing
//!
//! Archive root = skill directory root (`SKILL.md` at root). Text files only (UTF-8).

use base64::Engine;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use tar::{Builder, EntryType, Header};

/// Decoded archive payload size cap (before base64). Author: kejiqing
pub const MAX_SKILL_ARCHIVE_BYTES: usize = 2 * 1024 * 1024;
/// Soft cap for entire `skills_json` serialized UTF-8 length. Author: kejiqing
pub const MAX_SKILLS_JSON_BYTES: usize = 16 * 1024 * 1024;
/// Inline file text in preview responses. Author: kejiqing
pub const MAX_PREVIEW_TEXT_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillArchiveFormat {
    Tar,
    Tgz,
}

impl SkillArchiveFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tar => "tar",
            Self::Tgz => "tgz",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "tar" => Ok(Self::Tar),
            "tgz" | "tar.gz" | "gzip" => Ok(Self::Tgz),
            other => Err(format!(
                "skillArchiveFormat must be tar or tgz, got '{other}'"
            )),
        }
    }

    pub fn detect_from_bytes(bytes: &[u8]) -> Self {
        // gzip magic
        if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
            Self::Tgz
        } else {
            Self::Tar
        }
    }
}

#[derive(Debug, Clone)]
pub struct SkillFileEntry {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct SkillPackage {
    pub files: BTreeMap<String, String>,
}

impl SkillPackage {
    pub fn from_files(files: BTreeMap<String, String>) -> Result<Self, String> {
        validate_file_map(&files)?;
        Ok(Self { files })
    }

    pub fn skill_md(&self) -> &str {
        self.files.get("SKILL.md").map(String::as_str).unwrap_or("")
    }

    pub fn file_list(&self) -> Vec<SkillFileEntry> {
        self.files
            .iter()
            .map(|(path, content)| SkillFileEntry {
                path: path.clone(),
                content: content.clone(),
            })
            .collect()
    }

    pub fn pack(&self, format: SkillArchiveFormat) -> Result<Vec<u8>, String> {
        pack_files(&self.files, format)
    }

    pub fn pack_base64(
        &self,
        format: SkillArchiveFormat,
    ) -> Result<(String, SkillArchiveFormat), String> {
        let bytes = self.pack(format)?;
        Ok((
            base64::engine::general_purpose::STANDARD.encode(bytes),
            format,
        ))
    }
}

/// Validate and unpack archive bytes into a text file map. Author: kejiqing
pub fn unpack_archive_bytes(
    bytes: &[u8],
    format: Option<SkillArchiveFormat>,
) -> Result<SkillPackage, String> {
    if bytes.is_empty() {
        return Err("skill archive is empty".to_string());
    }
    if bytes.len() > MAX_SKILL_ARCHIVE_BYTES {
        return Err(format!(
            "skill archive exceeds {MAX_SKILL_ARCHIVE_BYTES} bytes (decoded)"
        ));
    }
    let format = format.unwrap_or_else(|| SkillArchiveFormat::detect_from_bytes(bytes));
    let tar_bytes: Vec<u8> = match format {
        SkillArchiveFormat::Tar => bytes.to_vec(),
        SkillArchiveFormat::Tgz => {
            let mut decoder = GzDecoder::new(bytes);
            let mut out = Vec::new();
            decoder
                .read_to_end(&mut out)
                .map_err(|e| format!("gunzip skill archive failed: {e}"))?;
            if out.len() > MAX_SKILL_ARCHIVE_BYTES {
                return Err(format!(
                    "skill archive exceeds {MAX_SKILL_ARCHIVE_BYTES} bytes after gunzip"
                ));
            }
            out
        }
    };
    let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
    let mut files = BTreeMap::new();
    let entries = archive
        .entries()
        .map_err(|e| format!("read tar entries failed: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("tar entry failed: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("tar path failed: {e}"))?
            .to_path_buf();
        let rel = normalize_archive_rel_path(&path)?;
        match entry.header().entry_type() {
            EntryType::Directory => continue,
            EntryType::Regular | EntryType::Continuous => {}
            other => {
                return Err(format!(
                    "unsupported tar entry type {:?} for '{rel}'",
                    other
                ));
            }
        }
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("read tar member '{rel}' failed: {e}"))?;
        if files.len() + 1 > 256 {
            return Err("skill archive has too many files (max 256)".to_string());
        }
        let text = bytes_to_utf8_text(&buf, &rel)?;
        if files.insert(rel.clone(), text).is_some() {
            return Err(format!("duplicate path in skill archive: {rel}"));
        }
    }
    validate_file_map(&files)?;
    Ok(SkillPackage { files })
}

pub fn unpack_archive_base64(
    b64: &str,
    format: Option<SkillArchiveFormat>,
) -> Result<SkillPackage, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("skillArchive base64 decode failed: {e}"))?;
    unpack_archive_bytes(&bytes, format)
}

pub fn pack_files(
    files: &BTreeMap<String, String>,
    format: SkillArchiveFormat,
) -> Result<Vec<u8>, String> {
    validate_file_map(files)?;
    let mut builder = Builder::new(Vec::new());
    for (rel, content) in files {
        let data = content.as_bytes();
        let mut header = Header::new_gnu();
        header
            .set_path(rel)
            .map_err(|e| format!("tar set_path '{rel}' failed: {e}"))?;
        header.set_size(data.len() as u64);
        header.set_mode(if looks_executable(rel, content) {
            0o755
        } else {
            0o644
        });
        header.set_entry_type(EntryType::Regular);
        header.set_cksum();
        builder
            .append(&header, data)
            .map_err(|e| format!("tar append '{rel}' failed: {e}"))?;
    }
    let tar_bytes = builder
        .into_inner()
        .map_err(|e| format!("finish tar failed: {e}"))?;
    if tar_bytes.len() > MAX_SKILL_ARCHIVE_BYTES {
        return Err(format!(
            "packed skill archive exceeds {MAX_SKILL_ARCHIVE_BYTES} bytes"
        ));
    }
    match format {
        SkillArchiveFormat::Tar => Ok(tar_bytes),
        SkillArchiveFormat::Tgz => {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(&tar_bytes)
                .map_err(|e| format!("gzip skill archive failed: {e}"))?;
            let gz = encoder
                .finish()
                .map_err(|e| format!("finish gzip failed: {e}"))?;
            if gz.len() > MAX_SKILL_ARCHIVE_BYTES {
                return Err(format!(
                    "gzipped skill archive exceeds {MAX_SKILL_ARCHIVE_BYTES} bytes"
                ));
            }
            Ok(gz)
        }
    }
}

pub fn package_from_skill_content(skill_content: &str) -> Result<SkillPackage, String> {
    let mut files = BTreeMap::new();
    files.insert("SKILL.md".to_string(), skill_content.to_string());
    SkillPackage::from_files(files)
}

/// Resolve a skills_json item into a package (archive preferred, else skillContent). Author: kejiqing
pub fn package_from_skills_json_item(item: &Value) -> Result<SkillPackage, String> {
    let obj = item
        .as_object()
        .ok_or_else(|| "skill item must be an object".to_string())?;
    if let Some(archive) = obj.get("skillArchive").and_then(Value::as_str) {
        if !archive.trim().is_empty() {
            let format = obj
                .get("skillArchiveFormat")
                .and_then(Value::as_str)
                .map(SkillArchiveFormat::parse)
                .transpose()?;
            return unpack_archive_base64(archive, format);
        }
    }
    let content = obj
        .get("skillContent")
        .and_then(Value::as_str)
        .ok_or_else(|| "skill item missing skillArchive and skillContent".to_string())?;
    package_from_skill_content(content)
}

/// Validate one skills_json array item. Author: kejiqing
pub fn validate_skills_json_item(item: &Value, index: usize) -> Result<(), String> {
    let obj = item
        .as_object()
        .ok_or_else(|| format!("skillsJson[{index}] must be a JSON object"))?;
    let name = obj
        .get("skillName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("skillsJson[{index}] missing skillName"))?;
    validate_skill_name(name)?;
    let has_archive = obj
        .get("skillArchive")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());
    let has_content = obj.contains_key("skillContent");
    if !has_archive && !has_content {
        return Err(format!(
            "skillsJson[{index}] requires skillArchive or skillContent"
        ));
    }
    if has_archive {
        if let Some(fmt) = obj.get("skillArchiveFormat").and_then(Value::as_str) {
            SkillArchiveFormat::parse(fmt)?;
        }
        // Full unpack validates structure / UTF-8 / SKILL.md
        package_from_skills_json_item(item).map_err(|e| format!("skillsJson[{index}]: {e}"))?;
    }
    Ok(())
}

pub fn validate_skills_json_value(skills: &Value) -> Result<(), String> {
    let arr = skills
        .as_array()
        .ok_or_else(|| "skillsJson must be a JSON array".to_string())?;
    let encoded =
        serde_json::to_string(skills).map_err(|e| format!("skillsJson serialize failed: {e}"))?;
    if encoded.len() > MAX_SKILLS_JSON_BYTES {
        return Err(format!(
            "skillsJson exceeds {MAX_SKILLS_JSON_BYTES} bytes when serialized"
        ));
    }
    for (i, item) in arr.iter().enumerate() {
        validate_skills_json_item(item, i)?;
    }
    Ok(())
}

pub fn validate_skill_name(skill_name: &str) -> Result<(), String> {
    if skill_name.trim().is_empty() {
        return Err("skillName cannot be empty".to_string());
    }
    if skill_name
        .chars()
        .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.'))
    {
        return Err("skillName only allows [a-zA-Z0-9._-]".to_string());
    }
    Ok(())
}

/// Build / update a skills_json item from a package (stores tgz + skillContent cache). Author: kejiqing
pub fn skill_item_from_package(
    skill_name: &str,
    package: &SkillPackage,
    enabled: Option<bool>,
) -> Result<Value, String> {
    validate_skill_name(skill_name)?;
    let (b64, format) = package.pack_base64(SkillArchiveFormat::Tgz)?;
    let mut obj = serde_json::Map::new();
    obj.insert("skillName".into(), json!(skill_name));
    obj.insert("skillContent".into(), json!(package.skill_md()));
    obj.insert("skillArchiveFormat".into(), json!(format.as_str()));
    obj.insert("skillArchive".into(), json!(b64));
    if let Some(en) = enabled {
        obj.insert("enabled".into(), json!(en));
    }
    Ok(Value::Object(obj))
}

/// Merge package into skills_json array by skillName. Author: kejiqing
pub fn merge_package_into_skills_json(
    skills_json: &mut Value,
    skill_name: &str,
    package: &SkillPackage,
    enabled: Option<bool>,
) -> Result<(), String> {
    if !skills_json.is_array() {
        *skills_json = json!([]);
    }
    let item = skill_item_from_package(skill_name, package, enabled)?;
    let arr = skills_json.as_array_mut().expect("skills_json is array");
    for existing in arr.iter_mut() {
        if existing.get("skillName").and_then(Value::as_str) == Some(skill_name) {
            let keep_enabled = enabled.or_else(|| existing.get("enabled").and_then(Value::as_bool));
            *existing = skill_item_from_package(skill_name, package, keep_enabled)?;
            return Ok(());
        }
    }
    arr.push(item);
    Ok(())
}

/// Write package files under `skill_dir` (creates dirs; sets +x on scripts). Author: kejiqing
pub fn materialize_package_to_dir(package: &SkillPackage, skill_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(skill_dir)
        .map_err(|e| format!("create skill dir {}: {e}", skill_dir.display()))?;
    for (rel, content) in &package.files {
        let dest = skill_dir.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        std::fs::write(&dest, content.as_bytes())
            .map_err(|e| format!("write {}: {e}", dest.display()))?;
        if looks_executable(rel, content) {
            set_executable(&dest)?;
        }
    }
    Ok(())
}

/// Guest materialize writes: relative path under project root + bytes. Author: kejiqing
pub fn package_guest_writes(
    package: &SkillPackage,
    skill_name: &str,
) -> Result<Vec<(PathBuf, Vec<u8>, bool)>, String> {
    let mut out = Vec::new();
    for (rel, content) in &package.files {
        let path = PathBuf::from(".claw/skills").join(skill_name).join(rel);
        out.push((
            path,
            content.as_bytes().to_vec(),
            looks_executable(rel, content),
        ));
    }
    Ok(out)
}

pub fn preview_entries(package: &SkillPackage) -> Vec<Value> {
    package
        .files
        .iter()
        .map(|(path, content)| {
            let size = content.len();
            let mut obj = json!({
                "path": path,
                "size": size,
            });
            if size <= MAX_PREVIEW_TEXT_BYTES {
                obj.as_object_mut()
                    .expect("object")
                    .insert("text".into(), json!(content));
            }
            obj
        })
        .collect()
}

fn validate_file_map(files: &BTreeMap<String, String>) -> Result<(), String> {
    if files.is_empty() {
        return Err("skill package has no files".to_string());
    }
    if !files.contains_key("SKILL.md") {
        return Err("skill package must contain SKILL.md at archive root".to_string());
    }
    let mut total = 0usize;
    for (rel, content) in files {
        normalize_archive_rel_path(Path::new(rel))?;
        total = total.saturating_add(content.len());
        if total > MAX_SKILL_ARCHIVE_BYTES {
            return Err(format!(
                "skill package text exceeds {MAX_SKILL_ARCHIVE_BYTES} bytes"
            ));
        }
        // Reject NULs as binary marker even if UTF-8.
        if content.as_bytes().contains(&0) {
            return Err(format!(
                "skill file '{rel}' contains NUL bytes (binary rejected)"
            ));
        }
    }
    Ok(())
}

fn normalize_archive_rel_path(path: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for comp in path.components() {
        match comp {
            Component::Normal(s) => {
                let s = s.to_string_lossy();
                if s.is_empty() || s == "." {
                    continue;
                }
                if s.contains('\0') {
                    return Err("path contains NUL".to_string());
                }
                parts.push(s.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!("path traversal not allowed: {}", path.display()));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "absolute paths not allowed in skill archive: {}",
                    path.display()
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err("empty path in skill archive".to_string());
    }
    Ok(parts.join("/"))
}

fn bytes_to_utf8_text(buf: &[u8], rel: &str) -> Result<String, String> {
    if buf.contains(&0) {
        return Err(format!(
            "skill file '{rel}' looks binary (NUL byte); binary assets are not supported"
        ));
    }
    String::from_utf8(buf.to_vec()).map_err(|_| {
        format!("skill file '{rel}' is not valid UTF-8; binary assets are not supported")
    })
}

fn looks_executable(rel: &str, content: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    if lower.ends_with(".sh") || lower.ends_with(".bash") {
        return true;
    }
    content.starts_with("#!")
}

fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(path, perms)
            .map_err(|e| format!("chmod +x {}: {e}", path.display()))?;
    }
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_tgz_with_scripts() {
        let mut files = BTreeMap::new();
        files.insert(
            "SKILL.md".into(),
            "---\nname: demo\ndescription: d\n---\n\n# Demo\nRun scripts/run.sh\n".into(),
        );
        files.insert("scripts/run.sh".into(), "#!/bin/sh\necho ok\n".into());
        let pkg = SkillPackage::from_files(files).unwrap();
        let bytes = pkg.pack(SkillArchiveFormat::Tgz).unwrap();
        let again = unpack_archive_bytes(&bytes, Some(SkillArchiveFormat::Tgz)).unwrap();
        assert!(again.files.contains_key("SKILL.md"));
        assert!(again.files.contains_key("scripts/run.sh"));
    }

    #[test]
    fn rejects_path_traversal() {
        let mut files = BTreeMap::new();
        files.insert("SKILL.md".into(), "# x\n".into());
        files.insert("../evil".into(), "x".into());
        assert!(SkillPackage::from_files(files).is_err());
    }

    #[test]
    fn rejects_missing_skill_md() {
        let mut files = BTreeMap::new();
        files.insert("readme.md".into(), "x".into());
        assert!(SkillPackage::from_files(files).is_err());
    }

    #[test]
    fn content_only_item_packages() {
        let item = json!({
            "skillName": "legacy",
            "skillContent": "# Legacy\n"
        });
        let pkg = package_from_skills_json_item(&item).unwrap();
        assert_eq!(pkg.skill_md(), "# Legacy\n");
    }

    #[test]
    fn validate_skills_json_accepts_archive_without_content() {
        let mut files = BTreeMap::new();
        files.insert("SKILL.md".into(), "# A\n".into());
        let pkg = SkillPackage::from_files(files).unwrap();
        let (b64, fmt) = pkg.pack_base64(SkillArchiveFormat::Tgz).unwrap();
        let skills = json!([{
            "skillName": "a",
            "skillArchive": b64,
            "skillArchiveFormat": fmt.as_str(),
        }]);
        validate_skills_json_value(&skills).unwrap();
    }
}
