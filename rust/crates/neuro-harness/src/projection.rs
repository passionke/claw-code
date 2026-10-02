//! Project metadata projection shared by engine profiles: instructions file and skills links.
//! Sources follow claw (`runtime::ProjectContext` under `CLAW_PROJECT_CONFIG_ROOT`). Author: kejiqing

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{HarnessError, HARNESS_DIR};

/// Instruction files + `.cursor/rules` discovered exactly like claw, the pool layout section
/// (`/claw_ds`, when `CLAW_GATEWAY_WORK_ROOT` is set) and the gateway `extraSession` block.
pub fn render_instructions(
    project_config_root: &Path,
    extra_session: Option<&Value>,
) -> Result<String, HarnessError> {
    let ctx = runtime::ProjectContext::discover_bounded(
        project_config_root,
        String::new(),
        Some(project_config_root),
    )
    .map_err(|e| {
        HarnessError::internal(format!(
            "discover instructions under {}: {e}",
            project_config_root.display()
        ))
    })?;
    let mut sections = Vec::new();
    for file in ctx.instruction_files.iter().chain(ctx.rule_files.iter()) {
        let body = file.content.trim();
        if body.is_empty() {
            continue;
        }
        let name = file
            .path
            .strip_prefix(project_config_root)
            .unwrap_or(&file.path)
            .display()
            .to_string();
        sections.push(format!("<!-- {name} -->\n{body}"));
    }
    if let Some(layout) = runtime::gateway_pool_layout_prompt_section() {
        sections.push(layout);
    }
    if let Some(extra) = extra_session {
        let body = serde_json::to_string_pretty(extra).unwrap_or_else(|_| extra.to_string());
        sections.push(format!(
            "## HTTP gateway extraSession\nSession-scoped JSON from the caller (tenant/user/workspace metadata, etc.). Use when relevant to the task.\n```json\n{body}\n```"
        ));
    }
    Ok(sections.join("\n\n") + "\n")
}

#[must_use]
pub fn instructions_path(session_root: &Path) -> PathBuf {
    session_root.join(HARNESS_DIR).join("instructions.md")
}

pub fn write_file(path: &Path, body: &[u8]) -> Result<(), HarnessError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| HarnessError::internal(format!("mkdir {}: {e}", parent.display())))?;
    }
    std::fs::write(path, body)
        .map_err(|e| HarnessError::internal(format!("write {}: {e}", path.display())))
}

/// Remove a file, symlink (without following it) or real directory; missing is fine.
fn remove_path(path: &Path) -> Result<(), HarnessError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
    .map_err(|e| HarnessError::internal(format!("clear {}: {e}", path.display())))
}

/// Point `link` at `target`, replacing whatever is there (file, dir link, stale link).
pub fn replace_symlink(link: &Path, target: &Path) -> Result<(), HarnessError> {
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| HarnessError::internal(format!("mkdir {}: {e}", parent.display())))?;
    }
    remove_path(link)?;
    std::os::unix::fs::symlink(target, link).map_err(|e| {
        HarnessError::internal(format!(
            "symlink {} -> {}: {e}",
            link.display(),
            target.display()
        ))
    })
}

/// Rebuild `dst_dir` as one symlink per skill directory in `src_dir` whose name passes `accept`.
/// Returns skipped names. Missing `src_dir` means no skills.
pub fn link_skills(
    src_dir: &Path,
    dst_dir: &Path,
    accept: impl Fn(&str) -> bool,
) -> Result<Vec<String>, HarnessError> {
    remove_path(dst_dir)?;
    std::fs::create_dir_all(dst_dir)
        .map_err(|e| HarnessError::internal(format!("mkdir {}: {e}", dst_dir.display())))?;
    let entries = match std::fs::read_dir(src_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(HarnessError::internal(format!(
                "read skills {}: {e}",
                src_dir.display()
            )))
        }
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().join("SKILL.md").is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    let mut skipped = Vec::new();
    for name in names {
        if accept(&name) {
            replace_symlink(&dst_dir.join(&name), &src_dir.join(&name))?;
        } else {
            skipped.push(name);
        }
    }
    Ok(skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn instructions_include_claude_md_rules_and_extra_session() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("CLAUDE.md"), "be terse").unwrap();
        std::fs::create_dir_all(root.join(".cursor/rules")).unwrap();
        std::fs::write(root.join(".cursor/rules/safety.mdc"), "no rm -rf").unwrap();
        let text = render_instructions(root, Some(&json!({"org_id":"o1"}))).unwrap();
        assert!(text.contains("be terse"), "{text}");
        assert!(text.contains("no rm -rf"), "{text}");
        assert!(text.contains("\"org_id\": \"o1\""), "{text}");
    }

    #[test]
    fn link_skills_filters_and_rebuilds() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        for name in ["good-one", "Bad_Name", "no-skill-md"] {
            std::fs::create_dir_all(src.join(name)).unwrap();
        }
        std::fs::write(src.join("good-one/SKILL.md"), "x").unwrap();
        std::fs::write(src.join("Bad_Name/SKILL.md"), "x").unwrap();
        let dst = dir.path().join("dst");
        let skipped = link_skills(&src, &dst, |n| !n.contains('_')).unwrap();
        assert_eq!(skipped, vec!["Bad_Name".to_string()]);
        assert!(dst.join("good-one/SKILL.md").is_file());
        assert!(!dst.join("no-skill-md").exists());
        // second run replaces cleanly
        link_skills(&src, &dst, |_| true).unwrap();
        assert!(dst.join("Bad_Name/SKILL.md").is_file());
        assert!(link_skills(&dir.path().join("missing"), &dst, |_| true)
            .unwrap()
            .is_empty());
    }
}
