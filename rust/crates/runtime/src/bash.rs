use std::io;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::runtime::Builder;
use tokio::time::timeout;

use crate::lane_events::{LaneEvent, ShipMergeMethod, ShipProvenance};
use crate::sandbox::{
    build_linux_sandbox_command, resolve_sandbox_status_for_request, FilesystemIsolationMode,
    SandboxConfig, SandboxStatus,
};
use crate::ConfigLoader;

/// Input schema for the built-in bash execution tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BashCommandInput {
    pub command: String,
    pub timeout: Option<u64>,
    pub description: Option<String>,
    #[serde(rename = "run_in_background")]
    pub run_in_background: Option<bool>,
    #[serde(rename = "dangerouslyDisableSandbox")]
    pub dangerously_disable_sandbox: Option<bool>,
    #[serde(rename = "namespaceRestrictions")]
    pub namespace_restrictions: Option<bool>,
    #[serde(rename = "isolateNetwork")]
    pub isolate_network: Option<bool>,
    #[serde(rename = "filesystemMode")]
    pub filesystem_mode: Option<FilesystemIsolationMode>,
    #[serde(rename = "allowedMounts")]
    pub allowed_mounts: Option<Vec<String>>,
}

/// Output returned from a bash tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BashCommandOutput {
    pub stdout: String,
    pub stderr: String,
    #[serde(rename = "rawOutputPath")]
    pub raw_output_path: Option<String>,
    pub interrupted: bool,
    #[serde(rename = "isImage")]
    pub is_image: Option<bool>,
    #[serde(rename = "backgroundTaskId")]
    pub background_task_id: Option<String>,
    #[serde(rename = "backgroundedByUser")]
    pub backgrounded_by_user: Option<bool>,
    #[serde(rename = "assistantAutoBackgrounded")]
    pub assistant_auto_backgrounded: Option<bool>,
    #[serde(rename = "dangerouslyDisableSandbox")]
    pub dangerously_disable_sandbox: Option<bool>,
    #[serde(rename = "returnCodeInterpretation")]
    pub return_code_interpretation: Option<String>,
    #[serde(rename = "noOutputExpected")]
    pub no_output_expected: Option<bool>,
    #[serde(rename = "structuredContent")]
    pub structured_content: Option<Vec<serde_json::Value>>,
    #[serde(rename = "persistedOutputPath")]
    pub persisted_output_path: Option<String>,
    #[serde(rename = "persistedOutputSize")]
    pub persisted_output_size: Option<u64>,
    #[serde(rename = "sandboxStatus")]
    pub sandbox_status: Option<SandboxStatus>,
}

std::thread_local! {
    static BASH_LINE_HOOK: std::cell::RefCell<Option<Arc<dyn Fn(&str) + Send + Sync>>> =
        const { std::cell::RefCell::new(None) };
}

/// Clears the bash line hook when dropped. Author: kejiqing
pub struct BashLineHookGuard;

impl Drop for BashLineHookGuard {
    fn drop(&mut self) {
        BASH_LINE_HOOK.with(|slot| *slot.borrow_mut() = None);
    }
}

/// Install a same-thread hook that sees each stdout/stderr chunk while bash runs.
/// Callers that do not install a hook keep the original buffered execution. Author: kejiqing
pub fn install_bash_line_hook(hook: Arc<dyn Fn(&str) + Send + Sync>) -> BashLineHookGuard {
    BASH_LINE_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
    BashLineHookGuard
}

fn current_bash_line_hook() -> Option<Arc<dyn Fn(&str) + Send + Sync>> {
    BASH_LINE_HOOK.with(|slot| slot.borrow().clone())
}

/// Executes a shell command with the requested sandbox settings.
pub fn execute_bash(input: BashCommandInput) -> io::Result<BashCommandOutput> {
    let cwd = crate::tool_effective_cwd()?;
    let sandbox_status = sandbox_status_for_input(&input, &cwd);

    if input.run_in_background.unwrap_or(false) {
        let mut child = prepare_command(&input.command, &cwd, &sandbox_status, false);
        let child = child
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        return Ok(BashCommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            raw_output_path: None,
            interrupted: false,
            is_image: None,
            background_task_id: Some(child.id().to_string()),
            backgrounded_by_user: Some(false),
            assistant_auto_backgrounded: Some(false),
            dangerously_disable_sandbox: input.dangerously_disable_sandbox,
            return_code_interpretation: None,
            no_output_expected: Some(true),
            structured_content: None,
            persisted_output_path: None,
            persisted_output_size: None,
            sandbox_status: Some(sandbox_status),
        });
    }

    if tokio::runtime::Handle::try_current().is_ok() {
        // We may be called from an async host (e.g. HTTP gateway). Reuse the
        // active runtime to avoid creating a nested Tokio runtime and panicking.
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(execute_bash_async(
                input,
                sandbox_status,
                cwd,
            ))
        })
    } else {
        // CLI/tests can invoke this from a plain sync context; create a local runtime.
        let runtime = Builder::new_current_thread().enable_all().build()?;
        runtime.block_on(execute_bash_async(input, sandbox_status, cwd))
    }
}

/// Detect git push to main and emit ship provenance event
fn detect_and_emit_ship_prepared(command: &str) {
    let trimmed = command.trim();
    // Simple detection: git push with main/master
    if trimmed.contains("git push") && (trimmed.contains("main") || trimmed.contains("master")) {
        // Emit ship.prepared event
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let provenance = ShipProvenance {
            source_branch: get_current_branch().unwrap_or_else(|| "unknown".to_string()),
            base_commit: get_head_commit().unwrap_or_default(),
            commit_count: 0, // Would need to calculate from range
            commit_range: "unknown..HEAD".to_string(),
            merge_method: ShipMergeMethod::DirectPush,
            actor: get_git_actor().unwrap_or_else(|| "unknown".to_string()),
            pr_number: None,
        };
        let _event = LaneEvent::ship_prepared(format!("{now}"), &provenance);
        // Log to stderr as interim routing before event stream integration
        eprintln!(
            "[ship.prepared] branch={} -> main, commits={}, actor={}",
            provenance.source_branch, provenance.commit_count, provenance.actor
        );
    }
}

fn get_current_branch() -> Option<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn get_head_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn get_git_actor() -> Option<String> {
    let name = Command::new("git")
        .args(["config", "user.name"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())?;
    Some(name)
}

/// When the model omits `timeout` on the bash tool, apply this ceiling (milliseconds) if set.
/// Example: `CLAW_BASH_DEFAULT_TIMEOUT_MS=120000` caps wall time without changing tool JSON.
/// Author: kejiqing
fn default_bash_timeout_ms_from_env() -> Option<u64> {
    std::env::var("CLAW_BASH_DEFAULT_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|&ms| ms > 0)
}

fn coalesce_bash_timeout(explicit: Option<u64>, default_ms: Option<u64>) -> Option<u64> {
    explicit.or(default_ms)
}

fn effective_bash_timeout_ms(input: &BashCommandInput) -> Option<u64> {
    coalesce_bash_timeout(input.timeout, default_bash_timeout_ms_from_env())
}

async fn execute_bash_async(
    input: BashCommandInput,
    sandbox_status: SandboxStatus,
    cwd: std::path::PathBuf,
) -> io::Result<BashCommandOutput> {
    // Detect and emit ship provenance for git push operations
    detect_and_emit_ship_prepared(&input.command);

    if let Some(hook) = current_bash_line_hook() {
        return execute_bash_streaming(input, sandbox_status, cwd, hook).await;
    }

    let mut command = prepare_tokio_command(&input.command, &cwd, &sandbox_status, true);

    let timeout_ms = effective_bash_timeout_ms(&input);
    let output_result = if let Some(timeout_ms) = timeout_ms {
        match timeout(Duration::from_millis(timeout_ms), command.output()).await {
            Ok(result) => (result?, false),
            Err(_) => {
                return Ok(BashCommandOutput {
                    stdout: String::new(),
                    stderr: format!("Command exceeded timeout of {timeout_ms} ms"),
                    raw_output_path: None,
                    interrupted: true,
                    is_image: None,
                    background_task_id: None,
                    backgrounded_by_user: None,
                    assistant_auto_backgrounded: None,
                    dangerously_disable_sandbox: input.dangerously_disable_sandbox,
                    return_code_interpretation: Some(String::from("timeout")),
                    no_output_expected: Some(true),
                    structured_content: None,
                    persisted_output_path: None,
                    persisted_output_size: None,
                    sandbox_status: Some(sandbox_status),
                });
            }
        }
    } else {
        (command.output().await?, false)
    };

    let (output, interrupted) = output_result;
    let stdout = truncate_output(&String::from_utf8_lossy(&output.stdout));
    let stderr = truncate_output(&String::from_utf8_lossy(&output.stderr));
    let no_output_expected = Some(stdout.trim().is_empty() && stderr.trim().is_empty());
    let return_code_interpretation = output.status.code().and_then(|code| {
        if code == 0 {
            None
        } else {
            Some(format!("exit_code:{code}"))
        }
    });

    Ok(BashCommandOutput {
        stdout,
        stderr,
        raw_output_path: None,
        interrupted,
        is_image: None,
        background_task_id: None,
        backgrounded_by_user: None,
        assistant_auto_backgrounded: None,
        dangerously_disable_sandbox: input.dangerously_disable_sandbox,
        return_code_interpretation,
        no_output_expected,
        structured_content: None,
        persisted_output_path: None,
        persisted_output_size: None,
        sandbox_status: Some(sandbox_status),
    })
}

/// Same result shape as [`execute_bash_async`], but forwards each pipe chunk to `hook`.
/// Author: kejiqing
async fn execute_bash_streaming(
    input: BashCommandInput,
    sandbox_status: SandboxStatus,
    cwd: std::path::PathBuf,
    hook: Arc<dyn Fn(&str) + Send + Sync>,
) -> io::Result<BashCommandOutput> {
    let mut command = prepare_tokio_command(&input.command, &cwd, &sandbox_status, true);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let timeout_ms = effective_bash_timeout_ms(&input);

    let collect = async {
        let stdout_acc = drain_pipe(stdout, Arc::clone(&hook));
        let stderr_acc = drain_pipe(stderr, hook);
        let status = child.wait();
        let (stdout_acc, stderr_acc, status) = tokio::join!(stdout_acc, stderr_acc, status);
        Ok::<_, io::Error>((status?, stdout_acc, stderr_acc))
    };

    let (status, stdout, stderr, interrupted, return_code_interpretation) =
        if let Some(timeout_ms) = timeout_ms {
            match timeout(Duration::from_millis(timeout_ms), collect).await {
                Ok(result) => {
                    let (status, stdout, stderr) = result?;
                    let return_code_interpretation = status.code().and_then(|code| {
                        if code == 0 {
                            None
                        } else {
                            Some(format!("exit_code:{code}"))
                        }
                    });
                    (status, stdout, stderr, false, return_code_interpretation)
                }
                Err(_) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Ok(BashCommandOutput {
                        stdout: String::new(),
                        stderr: format!("Command exceeded timeout of {timeout_ms} ms"),
                        raw_output_path: None,
                        interrupted: true,
                        is_image: None,
                        background_task_id: None,
                        backgrounded_by_user: None,
                        assistant_auto_backgrounded: None,
                        dangerously_disable_sandbox: input.dangerously_disable_sandbox,
                        return_code_interpretation: Some(String::from("timeout")),
                        no_output_expected: Some(true),
                        structured_content: None,
                        persisted_output_path: None,
                        persisted_output_size: None,
                        sandbox_status: Some(sandbox_status),
                    });
                }
            }
        } else {
            let (status, stdout, stderr) = collect.await?;
            let return_code_interpretation = status.code().and_then(|code| {
                if code == 0 {
                    None
                } else {
                    Some(format!("exit_code:{code}"))
                }
            });
            (status, stdout, stderr, false, return_code_interpretation)
        };
    let _ = status;
    let stdout = truncate_output(&stdout);
    let stderr = truncate_output(&stderr);
    let no_output_expected = Some(stdout.trim().is_empty() && stderr.trim().is_empty());
    Ok(BashCommandOutput {
        stdout,
        stderr,
        raw_output_path: None,
        interrupted,
        is_image: None,
        background_task_id: None,
        backgrounded_by_user: None,
        assistant_auto_backgrounded: None,
        dangerously_disable_sandbox: input.dangerously_disable_sandbox,
        return_code_interpretation,
        no_output_expected,
        structured_content: None,
        persisted_output_path: None,
        persisted_output_size: None,
        sandbox_status: Some(sandbox_status),
    })
}

async fn drain_pipe<R>(pipe: Option<R>, hook: Arc<dyn Fn(&str) + Send + Sync>) -> String
where
    R: tokio::io::AsyncRead + Unpin,
{
    let Some(pipe) = pipe else {
        return String::new();
    };
    let mut reader = BufReader::new(pipe);
    let mut acc = String::new();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = match reader.read_until(b'\n', &mut buf).await {
            Ok(n) => n,
            Err(_) => break,
        };
        if n == 0 {
            break;
        }
        let chunk = String::from_utf8_lossy(&buf).into_owned();
        hook(&chunk);
        acc.push_str(&chunk);
    }
    acc
}

fn sandbox_status_for_input(input: &BashCommandInput, cwd: &std::path::Path) -> SandboxStatus {
    let config = ConfigLoader::default_for(cwd).load().map_or_else(
        |_| SandboxConfig::default(),
        |runtime_config| runtime_config.sandbox().clone(),
    );
    let request = config.resolve_request(
        input.dangerously_disable_sandbox.map(|disabled| !disabled),
        input.namespace_restrictions,
        input.isolate_network,
        input.filesystem_mode,
        input.allowed_mounts.clone(),
    );
    resolve_sandbox_status_for_request(&request, cwd)
}

fn prepare_command(
    command: &str,
    cwd: &std::path::Path,
    sandbox_status: &SandboxStatus,
    create_dirs: bool,
) -> Command {
    if create_dirs {
        prepare_sandbox_dirs(cwd);
    }

    if let Some(launcher) = build_linux_sandbox_command(command, cwd, sandbox_status) {
        let mut prepared = Command::new(launcher.program);
        prepared.args(launcher.args);
        prepared.current_dir(cwd);
        prepared.envs(launcher.env);
        return prepared;
    }

    let mut prepared = Command::new("sh");
    prepared.arg("-lc").arg(command).current_dir(cwd);
    if sandbox_status.filesystem_active {
        prepared.env("HOME", cwd.join(".sandbox-home"));
        prepared.env("TMPDIR", cwd.join(".sandbox-tmp"));
    }
    prepared
}

fn prepare_tokio_command(
    command: &str,
    cwd: &std::path::Path,
    sandbox_status: &SandboxStatus,
    create_dirs: bool,
) -> TokioCommand {
    if create_dirs {
        prepare_sandbox_dirs(cwd);
    }

    if let Some(launcher) = build_linux_sandbox_command(command, cwd, sandbox_status) {
        let mut prepared = TokioCommand::new(launcher.program);
        prepared.args(launcher.args);
        prepared.current_dir(cwd);
        prepared.envs(launcher.env);
        return prepared;
    }

    let mut prepared = TokioCommand::new("sh");
    prepared.arg("-lc").arg(command).current_dir(cwd);
    if sandbox_status.filesystem_active {
        prepared.env("HOME", cwd.join(".sandbox-home"));
        prepared.env("TMPDIR", cwd.join(".sandbox-tmp"));
    }
    prepared
}

fn prepare_sandbox_dirs(cwd: &std::path::Path) {
    let _ = std::fs::create_dir_all(cwd.join(".sandbox-home"));
    let _ = std::fs::create_dir_all(cwd.join(".sandbox-tmp"));
}

#[cfg(test)]
mod tests {
    use super::{coalesce_bash_timeout, execute_bash, install_bash_line_hook, BashCommandInput};
    use crate::sandbox::FilesystemIsolationMode;

    fn bash_input(command: &str, timeout: Option<u64>) -> BashCommandInput {
        BashCommandInput {
            command: command.to_string(),
            timeout,
            description: None,
            run_in_background: Some(false),
            dangerously_disable_sandbox: Some(false),
            namespace_restrictions: Some(false),
            isolate_network: Some(false),
            filesystem_mode: Some(FilesystemIsolationMode::WorkspaceOnly),
            allowed_mounts: None,
        }
    }

    #[test]
    fn coalesce_bash_timeout_prefers_explicit() {
        assert_eq!(coalesce_bash_timeout(Some(5), Some(99)), Some(5));
        assert_eq!(coalesce_bash_timeout(None, Some(99)), Some(99));
        assert_eq!(coalesce_bash_timeout(None, None), None);
    }

    #[test]
    fn line_hook_sees_stdout_chunks() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let seen_hook = std::sync::Arc::clone(&seen);
        let _guard = install_bash_line_hook(std::sync::Arc::new(move |chunk| {
            seen_hook.lock().expect("hook lock").push_str(chunk);
        }));
        let output = execute_bash(bash_input("printf 'a\\nb\\n'", Some(5_000))).expect("bash");
        assert!(output.stdout.contains('a'));
        let got = seen.lock().expect("seen").clone();
        assert!(got.contains('a'), "hook missed stdout: {got:?}");
        assert!(got.contains('b'), "hook missed second line: {got:?}");
    }

    #[test]
    fn executes_simple_command() {
        let output = execute_bash(BashCommandInput {
            command: String::from("printf 'hello'"),
            timeout: Some(1_000),
            description: None,
            run_in_background: Some(false),
            dangerously_disable_sandbox: Some(false),
            namespace_restrictions: Some(false),
            isolate_network: Some(false),
            filesystem_mode: Some(FilesystemIsolationMode::WorkspaceOnly),
            allowed_mounts: None,
        })
        .expect("bash command should execute");

        assert_eq!(output.stdout, "hello");
        assert!(!output.interrupted);
        assert!(output.sandbox_status.is_some());
    }

    #[test]
    fn disables_sandbox_when_requested() {
        let output = execute_bash(BashCommandInput {
            command: String::from("printf 'hello'"),
            timeout: Some(1_000),
            description: None,
            run_in_background: Some(false),
            dangerously_disable_sandbox: Some(true),
            namespace_restrictions: None,
            isolate_network: None,
            filesystem_mode: None,
            allowed_mounts: None,
        })
        .expect("bash command should execute");

        assert!(!output.sandbox_status.expect("sandbox status").enabled);
    }

    /// `CLAW_BASH_DEFAULT_TIMEOUT_MS` only applies when the tool omits `timeout`; must not break
    /// normal short commands or override an explicit ceiling.
    #[cfg(unix)]
    #[test]
    fn bash_default_timeout_env_interrupts_long_sleep_without_explicit() {
        let _guard = crate::test_env_lock();
        std::env::set_var("CLAW_BASH_DEFAULT_TIMEOUT_MS", "280");
        let out = execute_bash(bash_input("sleep 30", None)).expect("execute");
        std::env::remove_var("CLAW_BASH_DEFAULT_TIMEOUT_MS");
        assert!(out.interrupted, "expected wall timeout");
        assert!(
            out.stderr.contains("280"),
            "stderr should mention ms cap: {}",
            out.stderr
        );
        assert_eq!(out.return_code_interpretation.as_deref(), Some("timeout"));
    }

    #[cfg(unix)]
    #[test]
    fn bash_default_timeout_env_allows_quick_command() {
        let _guard = crate::test_env_lock();
        std::env::set_var("CLAW_BASH_DEFAULT_TIMEOUT_MS", "200");
        let out = execute_bash(bash_input("printf 'ok'", None)).expect("execute");
        std::env::remove_var("CLAW_BASH_DEFAULT_TIMEOUT_MS");
        assert!(!out.interrupted);
        assert_eq!(out.stdout, "ok");
    }

    #[cfg(unix)]
    #[test]
    fn bash_explicit_timeout_overrides_tiny_default_env() {
        let _guard = crate::test_env_lock();
        std::env::set_var("CLAW_BASH_DEFAULT_TIMEOUT_MS", "80");
        let out = execute_bash(bash_input("sleep 0.35", Some(10_000))).expect("execute");
        std::env::remove_var("CLAW_BASH_DEFAULT_TIMEOUT_MS");
        assert!(
            !out.interrupted,
            "explicit timeout must win; stderr={}",
            out.stderr
        );
    }

    #[cfg(unix)]
    #[test]
    fn bash_default_timeout_env_whitespace_trimmed() {
        let _guard = crate::test_env_lock();
        std::env::set_var("CLAW_BASH_DEFAULT_TIMEOUT_MS", "  260  ");
        let out = execute_bash(bash_input("sleep 30", None)).expect("execute");
        std::env::remove_var("CLAW_BASH_DEFAULT_TIMEOUT_MS");
        assert!(out.interrupted);
        assert!(out.stderr.contains("260"), "stderr={}", out.stderr);
    }

    #[test]
    fn bash_invalid_or_zero_default_timeout_env_is_ignored() {
        let _guard = crate::test_env_lock();
        for v in ["0", "", "  ", "not-a-number"] {
            std::env::set_var("CLAW_BASH_DEFAULT_TIMEOUT_MS", v);
            let out = execute_bash(bash_input("printf 'z'", None)).expect("execute");
            assert!(
                !out.interrupted,
                "invalid/zero env should not cap; v={v:?} stderr={}",
                out.stderr
            );
            assert_eq!(out.stdout, "z");
        }
        std::env::remove_var("CLAW_BASH_DEFAULT_TIMEOUT_MS");
    }
}

/// Maximum output bytes before truncation (16 KiB, matching upstream).
const MAX_OUTPUT_BYTES: usize = 16_384;

/// Truncate output to `MAX_OUTPUT_BYTES`, appending a marker when trimmed.
fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_BYTES {
        return s.to_string();
    }
    // Find the last valid UTF-8 boundary at or before MAX_OUTPUT_BYTES
    let mut end = MAX_OUTPUT_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = s[..end].to_string();
    truncated.push_str("\n\n[output truncated — exceeded 16384 bytes]");
    truncated
}

#[cfg(test)]
mod truncation_tests {
    use super::*;

    #[test]
    fn short_output_unchanged() {
        let s = "hello world";
        assert_eq!(truncate_output(s), s);
    }

    #[test]
    fn long_output_truncated() {
        let s = "x".repeat(20_000);
        let result = truncate_output(&s);
        assert!(result.len() < 20_000);
        assert!(result.ends_with("[output truncated — exceeded 16384 bytes]"));
    }

    #[test]
    fn exact_boundary_unchanged() {
        let s = "a".repeat(MAX_OUTPUT_BYTES);
        assert_eq!(truncate_output(&s), s);
    }

    #[test]
    fn one_over_boundary_truncated() {
        let s = "a".repeat(MAX_OUTPUT_BYTES + 1);
        let result = truncate_output(&s);
        assert!(result.contains("[output truncated"));
    }
}
