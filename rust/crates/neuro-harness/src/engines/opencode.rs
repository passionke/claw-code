//! `neuro-opencode`: `opencode acp` with all config under the session root.
//! Contract: `docs/neuro-harness-contract.md` §8. Author: kejiqing

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};

use crate::profile::{AgentLaunch, EngineProfile, PrepareContext};
use crate::projection::{link_skills, write_file};
use crate::{HarnessError, HARNESS_DIR};

pub const DEFAULT_BIN: &str = "/usr/local/lib/neuro-engines/opencode/bin/opencode";
pub const BIN_ENV: &str = "NEURO_OPENCODE_BIN";
const PROVIDER: &str = "neurogate";

pub struct OpencodeProfile;

impl EngineProfile for OpencodeProfile {
    fn engine(&self) -> &'static str {
        "opencode"
    }

    fn prepare(&self, ctx: &PrepareContext<'_>) -> Result<AgentLaunch, HarnessError> {
        let root = ctx.session_root;
        let config_path = root.join(HARNESS_DIR).join("opencode.json");
        let body = serde_json::to_vec_pretty(&opencode_config(ctx))
            .map_err(|e| HarnessError::internal(format!("serialize opencode.json: {e}")))?;
        write_file(&config_path, &body)?;

        let skipped = link_skills(
            ctx.skills_dir,
            &root.join(".config/opencode/skills"),
            is_opencode_skill_name,
        )?;
        if !skipped.is_empty() {
            eprintln!(
                "[opencode] skills skipped (name must match ^[a-z0-9]+(-[a-z0-9]+)*$): {}",
                skipped.join(", ")
            );
        }

        Ok(AgentLaunch {
            command: super::engine_bin(BIN_ENV, DEFAULT_BIN),
            args: vec!["acp".to_string()],
            env: opencode_env(root, &config_path),
        })
    }
}

fn opencode_config(ctx: &PrepareContext<'_>) -> Value {
    let model_ref = format!("{PROVIDER}/{}", ctx.model);
    json!({
        "$schema": "https://opencode.ai/config.json",
        "autoupdate": false,
        "share": "disabled",
        "plugin": [],
        "model": model_ref,
        "small_model": model_ref,
        "provider": {
            PROVIDER: {
                "npm": "@ai-sdk/openai-compatible",
                "name": PROVIDER,
                "options": {"baseURL": ctx.openai_base_url, "apiKey": "{env:OPENAI_API_KEY}"},
                "models": {ctx.model: {"name": ctx.model}},
            }
        },
        "permission": {"edit": "allow", "bash": "allow", "webfetch": "allow"},
        "instructions": [ctx.instructions_path.to_string_lossy()],
    })
}

fn opencode_env(root: &Path, config_path: &Path) -> BTreeMap<String, String> {
    let p = |rel: &str| root.join(rel).to_string_lossy().to_string();
    let mut env = BTreeMap::new();
    for flag in [
        "OPENCODE_DISABLE_CLAUDE_CODE",
        "OPENCODE_DISABLE_AUTOUPDATE",
        "OPENCODE_DISABLE_MODELS_FETCH",
        "OPENCODE_DISABLE_DEFAULT_PLUGINS",
        "OPENCODE_DISABLE_LSP_DOWNLOAD",
        "OPENCODE_DISABLE_SHARE",
    ] {
        env.insert(flag.to_string(), "1".to_string());
    }
    env.insert(
        "OPENCODE_CONFIG".into(),
        config_path.to_string_lossy().to_string(),
    );
    env.insert("OPENCODE_DB".into(), p(".neuro-harness/opencode.db"));
    env.insert("HOME".into(), root.to_string_lossy().to_string());
    env.insert("XDG_CONFIG_HOME".into(), p(".config"));
    env.insert("XDG_DATA_HOME".into(), p(".local/share"));
    env.insert("XDG_STATE_HOME".into(), p(".local/state"));
    env.insert("XDG_CACHE_HOME".into(), p(".cache"));
    env
}

/// opencode only loads skills whose directory name matches `^[a-z0-9]+(-[a-z0-9]+)*$`.
fn is_opencode_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_name_rule() {
        for ok in ["a", "pdf", "data-report-2"] {
            assert!(is_opencode_skill_name(ok), "{ok}");
        }
        for bad in ["", "-a", "a-", "a--b", "Data", "a_b", "中文"] {
            assert!(!is_opencode_skill_name(bad), "{bad}");
        }
    }

    #[test]
    fn prepare_writes_config_and_env_under_session_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sess");
        let pcr = dir.path().join("pcr");
        std::fs::create_dir_all(pcr.join(".claw/skills/sql-helper")).unwrap();
        std::fs::write(pcr.join(".claw/skills/sql-helper/SKILL.md"), "x").unwrap();
        let instructions = root.join(".neuro-harness/instructions.md");
        let launch = OpencodeProfile
            .prepare(&PrepareContext {
                session_root: &root,
                project_config_root: &pcr,
                model: "m1",
                openai_base_url: "http://tap:8080",
                instructions_path: &instructions,
                skills_dir: &pcr.join(".claw/skills"),
            })
            .unwrap();
        assert_eq!(launch.args, vec!["acp"]);
        assert_eq!(
            launch.env["OPENCODE_DB"],
            root.join(".neuro-harness/opencode.db").to_string_lossy()
        );
        assert_eq!(
            launch.env["XDG_CONFIG_HOME"],
            root.join(".config").to_string_lossy()
        );
        let cfg: Value = serde_json::from_slice(
            &std::fs::read(root.join(".neuro-harness/opencode.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(cfg["model"], "neurogate/m1");
        assert_eq!(
            cfg["provider"]["neurogate"]["options"]["baseURL"],
            "http://tap:8080"
        );
        assert_eq!(
            cfg["instructions"][0],
            instructions.to_string_lossy().as_ref()
        );
        assert!(root
            .join(".config/opencode/skills/sql-helper/SKILL.md")
            .is_file());
    }
}
