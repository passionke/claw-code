//! Preflight pipeline runner: scope filter, builtins, subprocess SPI, effects applier. Author: kejiqing

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use preflight_spi::{
    default_runtime_pipeline_steps, filter_step_indices, merge_language_pipeline_into_steps,
    normalize_pipeline_steps, parse_pipeline_value, validate_spi_request,
    validate_subprocess_response, PreflightEffect, PreflightFilterContext, PreflightImpl,
    PreflightPipelineConfig, PreflightRequestContext, PreflightResponseStatus, PreflightScope,
    PreflightSpiRequest, PreflightSpiResponse, PreflightStep, BUILTIN_SQLBOT_MCP_START,
    BUILTIN_TURN_LANGUAGE, SPI_VERSION,
};
use runtime::{ContentBlock, ConversationMessage, MessageRole, Session};
use serde_json::Value;

use crate::project_language_pipeline::{resolve_language_pipeline_config, LanguagePipelineConfig};
use crate::turn_language::{
    inject_language_into_system_prompt, persist_turn_language, TurnLanguageFile,
};
use crate::{DirectToolExecutor, GatewaySolveTurnError, HTTP_INTERNAL};

const DEFAULT_SUBPROCESS_TIMEOUT_SECS: u64 = 120;
const DEFAULT_SUBPROCESS_MAX_OUTPUT_BYTES: usize = 1024 * 1024;

fn subprocess_timeout_secs() -> u64 {
    std::env::var("CLAW_PREFLIGHT_SUBPROCESS_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_SUBPROCESS_TIMEOUT_SECS)
}

fn subprocess_max_output_bytes() -> usize {
    std::env::var("CLAW_PREFLIGHT_SUBPROCESS_MAX_OUTPUT_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_SUBPROCESS_MAX_OUTPUT_BYTES)
}

/// Inputs for one solve-turn preflight pass.
pub struct PreflightRunParams<'a> {
    pub session_home: &'a Path,
    pub session: &'a mut Session,
    pub system_prompt: &'a mut Vec<String>,
    pub executor: &'a mut DirectToolExecutor,
    pub is_continuation: bool,
    pub user_prompt: &'a str,
    pub turn_id: &'a str,
    pub session_id: &'a str,
    pub model: &'a str,
    pub extra_session: Option<Value>,
}

fn err(status: u16, msg: impl Into<String>) -> GatewaySolveTurnError {
    GatewaySolveTurnError {
        status,
        message: msg.into(),
    }
}

/// Whether session-first-turn steps are already reflected in the session.
#[must_use]
pub fn session_first_turn_preflight_satisfied(
    _session_home: &Path,
    session: &Session,
    steps: &[PreflightStep],
) -> bool {
    for step in steps {
        if step.scope != PreflightScope::SessionFirstTurn {
            continue;
        }
        match step.plugin_id.as_str() {
            BUILTIN_SQLBOT_MCP_START => {
                if crate::sqlbot_preflight::sqlbot_query_context_from_session(session).is_none() {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

/// Apply declarative effects (runtime invariants: no raw message JSON from subprocess).
pub fn apply_preflight_effects(
    session_home: &Path,
    session: &mut Session,
    system_prompt: &mut Vec<String>,
    effects: &[PreflightEffect],
    turn_id: &str,
) -> Result<(), GatewaySolveTurnError> {
    for effect in effects {
        match effect {
            PreflightEffect::LockLanguage { language, reason } => {
                let lang = language.trim();
                if lang.is_empty() {
                    continue;
                }
                let file = TurnLanguageFile {
                    turn_id: turn_id.to_string(),
                    language: lang.to_string(),
                    reason: reason.clone(),
                    prior_turns_used: 0,
                    source: "preflight".to_string(),
                    updated_at_ms: now_ms(),
                };
                persist_turn_language(session_home, &file)
                    .map_err(|e| err(HTTP_INTERNAL, format!("persist turn-language: {e}")))?;
                inject_language_into_system_prompt(system_prompt, lang);
            }
            PreflightEffect::WriteSessionFile { rel_path, content } => {
                let rel = rel_path.trim_start_matches('/');
                let path = session_home.join(rel);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        err(
                            HTTP_INTERNAL,
                            format!("preflight mkdir {}: {e}", parent.display()),
                        )
                    })?;
                }
                std::fs::write(&path, content).map_err(|e| {
                    err(
                        HTTP_INTERNAL,
                        format!("preflight write {}: {e}", path.display()),
                    )
                })?;
            }
            PreflightEffect::AppendSystemPromptSection { markdown } => {
                let t = markdown.trim();
                if !t.is_empty() {
                    system_prompt.push(t.to_string());
                }
            }
            PreflightEffect::AppendTranscriptSummary { text } => {
                session
                    .push_message(ConversationMessage {
                        role: MessageRole::Assistant,
                        blocks: vec![ContentBlock::Text { text: text.clone() }],
                        usage: None,
                    })
                    .map_err(|e| {
                        err(
                            HTTP_INTERNAL,
                            format!("preflight appendTranscriptSummary: {e}"),
                        )
                    })?;
            }
            PreflightEffect::InjectToolExchange {
                tool_name,
                input,
                output,
                is_error,
            } => {
                let tool_use_id = format!("claw_preflight_{tool_name}_{}", now_ms());
                session
                    .push_message(ConversationMessage {
                        role: MessageRole::Assistant,
                        blocks: vec![ContentBlock::ToolUse {
                            id: tool_use_id.clone(),
                            name: tool_name.clone(),
                            input: input.clone(),
                        }],
                        usage: None,
                    })
                    .map_err(|e| err(HTTP_INTERNAL, format!("preflight tool_use: {e}")))?;
                session
                    .push_message(ConversationMessage::tool_result(
                        tool_use_id,
                        tool_name,
                        output.clone(),
                        *is_error,
                    ))
                    .map_err(|e| err(HTTP_INTERNAL, format!("preflight tool_result: {e}")))?;
            }
        }
    }
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn prior_user_prompts_excluding_current(
    session_home: &Path,
    max_turns: usize,
    max_chars: usize,
) -> Vec<String> {
    let (block, _) =
        crate::turn_language::collect_prior_user_prompts(session_home, max_turns, max_chars);
    if block == "(none)" {
        return vec![];
    }
    block
        .lines()
        .filter_map(|line| {
            let rest = line.split_once(']')?.1.trim();
            if rest.is_empty() {
                None
            } else {
                Some(rest.to_string())
            }
        })
        .collect()
}

fn build_spi_request(params: &PreflightRunParams<'_>, step: &PreflightStep) -> PreflightSpiRequest {
    let pipeline = resolve_language_pipeline_config(params.session_home);
    let priors = prior_user_prompts_excluding_current(
        params.session_home,
        pipeline.language_inference_prior_turns,
        pipeline.language_inference_prior_max_chars,
    );
    PreflightSpiRequest {
        spi_version: SPI_VERSION.to_string(),
        step: step.clone(),
        context: PreflightRequestContext {
            session_id: params.session_id.to_string(),
            turn_id: params.turn_id.to_string(),
            work_dir: params.session_home.display().to_string(),
            is_continuation: params.is_continuation,
            user_prompt: params.user_prompt.to_string(),
            prior_user_prompts: priors,
            extra_session: params.extra_session.clone().unwrap_or(Value::Null),
            model: params.model.to_string(),
        },
        artifacts: vec![
            ".claw/gateway-solve-session.jsonl".to_string(),
            ".claw/turn-language.json".to_string(),
        ],
    }
}

fn run_subprocess_preflight(
    step: &PreflightStep,
    request: &PreflightSpiRequest,
) -> Result<PreflightSpiResponse, GatewaySolveTurnError> {
    validate_spi_request(request).map_err(|e| err(HTTP_INTERNAL, e))?;
    let command = match step.r#impl.as_ref() {
        Some(PreflightImpl::Subprocess { command }) => command,
        _ => {
            return Err(err(
                HTTP_INTERNAL,
                format!(
                    "subprocess preflight {} missing impl.command",
                    step.plugin_id
                ),
            ));
        }
    };
    if command.is_empty() {
        return Err(err(HTTP_INTERNAL, "subprocess command is empty"));
    }
    let program = &command[0];
    let args: Vec<&str> = command.iter().skip(1).map(String::as_str).collect();
    let stdin_json = serde_json::to_string(request)
        .map_err(|e| err(HTTP_INTERNAL, format!("encode SPI: {e}")))?;

    let mut child = Command::new(program)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| err(HTTP_INTERNAL, format!("spawn preflight {program}: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_json.as_bytes())
            .map_err(|e| err(HTTP_INTERNAL, format!("preflight stdin: {e}")))?;
    }

    let timeout = Duration::from_secs(subprocess_timeout_secs());
    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| err(HTTP_INTERNAL, format!("preflight wait: {e}")))?
        {
            let mut stdout = Vec::new();
            if let Some(mut out) = child.stdout.take() {
                use std::io::Read;
                let _ = out.read_to_end(&mut stdout);
            }
            if stdout.len() > subprocess_max_output_bytes() {
                return Err(err(HTTP_INTERNAL, "preflight stdout too large"));
            }
            if !status.success() {
                return Err(err(
                    HTTP_INTERNAL,
                    format!("preflight exit {}", status.code().unwrap_or(-1)),
                ));
            }
            let parsed: PreflightSpiResponse = serde_json::from_slice(&stdout).map_err(|e| {
                err(
                    HTTP_INTERNAL,
                    format!(
                        "preflight stdout JSON: {e}; raw={}",
                        String::from_utf8_lossy(&stdout)
                    ),
                )
            })?;
            validate_subprocess_response(&parsed).map_err(|e| err(HTTP_INTERNAL, e))?;
            return Ok(parsed);
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            return Err(err(HTTP_INTERNAL, "preflight subprocess timed out"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn run_builtin_turn_language(
    params: &PreflightRunParams<'_>,
    step: &PreflightStep,
) -> Result<Vec<PreflightEffect>, GatewaySolveTurnError> {
    let pipeline_cfg: LanguagePipelineConfig =
        if step.config.is_object() && !step.config.as_object().unwrap().is_empty() {
            serde_json::from_value(step.config.clone())
                .map_err(|e| err(HTTP_INTERNAL, format!("turn_language step config: {e}")))?
        } else {
            resolve_language_pipeline_config(params.session_home)
        };
    let (prior_block, _prior_turns_used) =
        crate::turn_language::collect_prior_user_prompts_excluding(
            params.session_home,
            pipeline_cfg.language_inference_prior_turns,
            pipeline_cfg.language_inference_prior_max_chars,
            params.user_prompt,
        );
    let user_message = crate::project_language_pipeline::render_language_inference_prompt(
        &pipeline_cfg.language_inference_prompt,
        &prior_block,
        params.user_prompt.trim(),
    );
    let language = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            crate::turn_language::infer_turn_language_only(
                &user_message,
                params.model,
                params.session_id,
            )
            .await
        })
    })?;
    Ok(vec![PreflightEffect::LockLanguage {
        language,
        reason: Some("inference".to_string()),
    }])
}

fn resolve_step_impl(step: &PreflightStep) -> PreflightImpl {
    step.r#impl.clone().unwrap_or_else(|| {
        if step.plugin_id == BUILTIN_TURN_LANGUAGE {
            PreflightImpl::Builtin {
                handler: BUILTIN_TURN_LANGUAGE.to_string(),
            }
        } else if step.plugin_id == BUILTIN_SQLBOT_MCP_START {
            PreflightImpl::Builtin {
                handler: BUILTIN_SQLBOT_MCP_START.to_string(),
            }
        } else {
            PreflightImpl::Subprocess { command: vec![] }
        }
    })
}

/// Report from one preflight pipeline run.
#[derive(Debug, Clone, Copy, Default)]
pub struct PreflightRunReport {
    pub ran_session_first_turn: bool,
}

/// Resolve executable steps for one turn (`default_when_empty` only when no project file exists).
#[must_use]
pub fn resolve_pipeline_steps_for_run(
    pipeline: &PreflightPipelineConfig,
    language_pipeline_json: &Value,
    default_when_empty: bool,
) -> Vec<PreflightStep> {
    let mut steps = normalize_pipeline_steps(pipeline);
    if steps.is_empty() && default_when_empty {
        steps = default_runtime_pipeline_steps();
    }
    merge_language_pipeline_into_steps(steps, language_pipeline_json)
}

/// Run configured preflight steps for this turn.
pub fn run_preflight_pipeline(
    pipeline: &PreflightPipelineConfig,
    language_pipeline_json: &Value,
    params: PreflightRunParams<'_>,
    default_when_empty: bool,
) -> Result<PreflightRunReport, GatewaySolveTurnError> {
    let steps =
        resolve_pipeline_steps_for_run(pipeline, language_pipeline_json, default_when_empty);

    let session_first_satisfied =
        session_first_turn_preflight_satisfied(params.session_home, params.session, &steps);
    let filter_ctx = PreflightFilterContext {
        is_continuation: params.is_continuation,
        session_first_turn_satisfied: session_first_satisfied,
    };
    let indices = filter_step_indices(&steps, filter_ctx);
    let ran_session_first_turn = indices
        .iter()
        .any(|&idx| steps[idx].scope == PreflightScope::SessionFirstTurn);

    for idx in indices {
        let step = &steps[idx];
        let impl_kind = resolve_step_impl(step);
        match impl_kind {
            PreflightImpl::Builtin { handler } => match handler.as_str() {
                BUILTIN_TURN_LANGUAGE => {
                    let effects = run_builtin_turn_language(&params, step)?;
                    apply_preflight_effects(
                        params.session_home,
                        params.session,
                        params.system_prompt,
                        &effects,
                        params.turn_id,
                    )?;
                }
                BUILTIN_SQLBOT_MCP_START => {
                    crate::sqlbot_preflight::run_sqlbot_preflight(
                        params.session_home,
                        params.session,
                        params.executor,
                    )?;
                }
                other => {
                    return Err(err(
                        HTTP_INTERNAL,
                        format!("unknown builtin preflight handler {other:?}"),
                    ));
                }
            },
            PreflightImpl::Subprocess { .. } => {
                let request = build_spi_request(&params, step);
                let response = run_subprocess_preflight(step, &request)?;
                if response.status == PreflightResponseStatus::Skip {
                    continue;
                }
                apply_preflight_effects(
                    params.session_home,
                    params.session,
                    params.system_prompt,
                    &response.effects,
                    params.turn_id,
                )?;
            }
        }
    }
    Ok(PreflightRunReport {
        ran_session_first_turn,
    })
}

/// Resolve pipeline from materialized JSON value (file or DB).
#[must_use]
pub fn pipeline_from_value(value: &Value) -> PreflightPipelineConfig {
    parse_pipeline_value(value).unwrap_or(PreflightPipelineConfig {
        steps: vec![],
        kinds: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use preflight_spi::PreflightScope;
    use serde_json::json;
    use std::fs;

    #[test]
    fn resolve_pipeline_steps_respects_default_when_empty_flag() {
        use preflight_spi::{parse_pipeline_value, PreflightPipelineConfig, BUILTIN_TURN_LANGUAGE};
        let empty = PreflightPipelineConfig {
            steps: vec![],
            kinds: vec![],
        };
        let lp = json!({"languageInferencePrompt": "x"});
        let no_default = resolve_pipeline_steps_for_run(&empty, &lp, false);
        assert!(
            no_default.is_empty(),
            "default_when_empty=false must not inject turn_language"
        );
        let with_default = resolve_pipeline_steps_for_run(&empty, &json!({}), true);
        assert_eq!(with_default.len(), 1);
        assert_eq!(with_default[0].plugin_id, BUILTIN_TURN_LANGUAGE);

        let sqlbot = parse_pipeline_value(&json!({"kind":"sqlbot_mcp_start"})).unwrap();
        let explicit = resolve_pipeline_steps_for_run(&sqlbot, &json!({}), false);
        assert_eq!(explicit.len(), 2);
        assert_eq!(explicit[0].plugin_id, BUILTIN_TURN_LANGUAGE);
    }

    #[test]
    fn language_pipeline_json_only_merges_into_existing_turn_language_step() {
        use preflight_spi::{PreflightPipelineConfig, BUILTIN_TURN_LANGUAGE};
        let empty = PreflightPipelineConfig {
            steps: vec![],
            kinds: vec![],
        };
        let lp = json!({"languageInferencePrompt": "merged"});
        let no_step = resolve_pipeline_steps_for_run(&empty, &lp, false);
        assert!(
            no_step.is_empty(),
            "language_pipeline_json alone must not create steps"
        );

        let with_default = resolve_pipeline_steps_for_run(&empty, &lp, true);
        assert_eq!(with_default.len(), 1);
        assert_eq!(with_default[0].plugin_id, BUILTIN_TURN_LANGUAGE);
        assert_eq!(
            with_default[0]
                .config
                .get("languageInferencePrompt")
                .and_then(Value::as_str),
            Some("merged")
        );
    }

    #[test]
    fn apply_lock_language_writes_sidecar_and_system_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new();
        let mut system_prompt = vec!["Use [LANG_TAG].".to_string()];
        let effects = vec![PreflightEffect::LockLanguage {
            language: "Thai".into(),
            reason: None,
        }];
        apply_preflight_effects(dir.path(), &mut session, &mut system_prompt, &effects, "t1")
            .unwrap();
        assert_eq!(session.messages.len(), 0);
        assert!(system_prompt[1].contains("Thai"));
        let sidecar = dir.path().join(".claw/turn-language.json");
        let raw = fs::read_to_string(sidecar).unwrap();
        assert!(raw.contains("Thai"));
    }

    #[test]
    fn apply_transcript_summary_appends_assistant_text_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new();
        let mut system_prompt = vec![];
        apply_preflight_effects(
            dir.path(),
            &mut session,
            &mut system_prompt,
            &[PreflightEffect::AppendTranscriptSummary {
                text: "note".into(),
            }],
            "t1",
        )
        .unwrap();
        assert_eq!(session.messages.len(), 1);
        assert!(matches!(
            session.messages[0].blocks.first(),
            Some(ContentBlock::Text { .. })
        ));
    }

    #[test]
    fn scope_filter_matrix() {
        use preflight_spi::{filter_step_indices, PreflightFilterContext};
        let steps = vec![
            PreflightStep {
                plugin_id: BUILTIN_TURN_LANGUAGE.into(),
                scope: PreflightScope::EveryTurn,
                r#impl: None,
                config: json!({}),
            },
            PreflightStep {
                plugin_id: BUILTIN_SQLBOT_MCP_START.into(),
                scope: PreflightScope::SessionFirstTurn,
                r#impl: None,
                config: json!({}),
            },
        ];
        let ctx_run = PreflightFilterContext {
            is_continuation: false,
            session_first_turn_satisfied: false,
        };
        assert_eq!(filter_step_indices(&steps, ctx_run), vec![0, 1]);
        let ctx_cont = PreflightFilterContext {
            is_continuation: true,
            session_first_turn_satisfied: false,
        };
        assert_eq!(filter_step_indices(&steps, ctx_cont), vec![0]);
        let ctx_sat = PreflightFilterContext {
            is_continuation: false,
            session_first_turn_satisfied: true,
        };
        assert_eq!(filter_step_indices(&steps, ctx_sat), vec![0]);
    }

    #[test]
    fn inject_tool_exchange_pairs_tool_use_and_result() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new();
        let mut system_prompt = vec![];
        apply_preflight_effects(
            dir.path(),
            &mut session,
            &mut system_prompt,
            &[PreflightEffect::InjectToolExchange {
                tool_name: "mcp_start".into(),
                input: "{}".into(),
                output: r#"{"ok":true}"#.into(),
                is_error: false,
            }],
            "t1",
        )
        .unwrap();
        assert_eq!(session.messages.len(), 2);
        let use_id = match &session.messages[0].blocks[0] {
            ContentBlock::ToolUse { id, .. } => id.clone(),
            _ => panic!("expected tool use"),
        };
        match &session.messages[1].blocks[0] {
            ContentBlock::ToolResult { tool_use_id, .. } => assert_eq!(tool_use_id, &use_id),
            _ => panic!("expected tool result"),
        }
    }

    #[test]
    fn subprocess_rejects_inject_tool_exchange_at_runner() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("bad.sh");
        fs::write(
            &script,
            r#"#!/bin/sh
cat <<'EOF'
{"status":"ok","effects":[{"type":"injectToolExchange","toolName":"x","input":"{}","output":"{}"}]}
EOF
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let step = PreflightStep {
            plugin_id: "test".into(),
            scope: PreflightScope::EveryTurn,
            r#impl: Some(PreflightImpl::Subprocess {
                command: vec![script.display().to_string()],
            }),
            config: json!({}),
        };
        let req = PreflightSpiRequest {
            spi_version: SPI_VERSION.into(),
            step: step.clone(),
            context: PreflightRequestContext {
                session_id: "s".into(),
                turn_id: "t".into(),
                work_dir: dir.path().display().to_string(),
                is_continuation: false,
                user_prompt: "hi".into(),
                prior_user_prompts: vec![],
                extra_session: json!({}),
                model: "m".into(),
            },
            artifacts: vec![],
        };
        assert!(run_subprocess_preflight(&step, &req).is_err());
    }

    #[test]
    fn subprocess_ok_echo_json() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("ok.sh");
        fs::write(
            &script,
            r#"#!/bin/sh
cat <<'EOF'
{"status":"ok","effects":[{"type":"appendSystemPromptSection","markdown":"SPI section"}]}
EOF
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let step = PreflightStep {
            plugin_id: "test".into(),
            scope: PreflightScope::EveryTurn,
            r#impl: Some(PreflightImpl::Subprocess {
                command: vec![script.display().to_string()],
            }),
            config: json!({}),
        };
        let req = PreflightSpiRequest {
            spi_version: SPI_VERSION.into(),
            step: step.clone(),
            context: PreflightRequestContext {
                session_id: "s".into(),
                turn_id: "t".into(),
                work_dir: dir.path().display().to_string(),
                is_continuation: false,
                user_prompt: "hi".into(),
                prior_user_prompts: vec![],
                extra_session: json!({}),
                model: "m".into(),
            },
            artifacts: vec![],
        };
        let resp = run_subprocess_preflight(&step, &req).unwrap();
        assert_eq!(resp.effects.len(), 1);
    }
}
