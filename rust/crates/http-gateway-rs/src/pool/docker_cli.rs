//! Invoke `docker` / `podman` CLI. Author: kejiqing

use std::process::{Command as StdCommand, Stdio};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Fail fast when the bundled `docker.io` client (API 1.41) cannot talk to a modern Engine (≥1.44).
pub fn probe_container_runtime_cli(bin: &str) -> Result<(), String> {
    let output = StdCommand::new(bin)
        .args(["version", "--format", "{{.Client.APIVersion}}"])
        .output()
        .map_err(|e| format!("{bin} version probe failed: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stderr}{stdout}");
    if combined.contains("too old") || combined.contains("Minimum supported API version") {
        return Err(format!(
            "{bin} client API is too old for this container engine ({combined}); \
             upgrade the host claw-pool-daemon binary (deploy/stack/.linux-artifacts/release/claw-pool-daemon)"
        ));
    }
    Err(format!("{bin} version probe failed: {combined}"))
}

/// Short-lived CLI calls (`run`, `rm`, `kill`). `kill_on_drop` tears down the client if the
/// awaiting task is cancelled (e.g. async solve abort), so the runtime does not leave a stuck
/// `docker exec` child on the host.
pub async fn runtime_exec(bin: &str, args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new(bin)
        .args(args)
        .kill_on_drop(true)
        .output()
        .await
}

/// `docker exec -i` with stdin bytes (materialize session files as worker uid). Author: kejiqing
pub async fn runtime_exec_stdin(
    bin: &str,
    args: &[&str],
    stdin_bytes: &[u8],
) -> std::io::Result<std::process::Output> {
    use tokio::io::AsyncWriteExt;

    let mut child = Command::new(bin)
        .args(args)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(stdin_bytes).await?;
    }
    child.wait_with_output().await
}

fn argv_summary(args: &[&str], max_bytes: usize) -> String {
    let s = args.join(" ");
    if s.len() <= max_bytes {
        return s;
    }
    let mut start = s.len().saturating_sub(max_bytes);
    while start > 0 && !s.is_char_boundary(start) {
        start -= 1;
    }
    format!("…{}", &s[start..])
}

/// `docker exec` (long-running): stream stderr to tracing; stream stdout lines to `on_stdout_line`.
pub async fn runtime_exec_with_live_streams(
    bin: &str,
    args: &[&str],
    request_id: Option<&str>,
    on_stdout_line: Option<Arc<dyn Fn(String) + Send + Sync>>,
) -> std::io::Result<std::process::Output> {
    tracing::debug!(
        target: "claw_gateway_pool",
        component = "docker_cli",
        phase = "exec_spawn",
        %bin,
        argv_summary = %argv_summary(args, 1800),
        request_id = request_id.unwrap_or(""),
        "spawning docker/podman exec (stdout/stderr streamed)"
    );
    let mut child = Command::new(bin)
        .args(args)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stderr = child.stderr.take().expect("stderr piped");
    let stdout = child.stdout.take().expect("stdout piped");

    let rid = request_id.map(std::string::ToString::to_string);
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        let mut acc = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    acc.push_str(&line);
                    let trimmed = line.trim_end();
                    if !trimmed.is_empty() {
                        match &rid {
                            Some(id) => {
                                tracing::info!(
                                    target: "claw_gateway_solve",
                                    component = "worker_stderr",
                                    request_id = %id,
                                    line = %trimmed,
                                    "claw gateway-solve-once stderr line"
                                );
                            }
                            None => {
                                tracing::info!(
                                    target: "claw_gateway_solve",
                                    component = "worker_stderr",
                                    line = %trimmed,
                                    "claw gateway-solve-once stderr line"
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        target: "claw_gateway_pool",
                        component = "docker_cli",
                        phase = "exec_stderr_reader_done",
                        error = %e,
                        "gateway docker exec stderr stream ended"
                    );
                    break;
                }
            }
        }
        acc
    });

    let stdout_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let mut acc = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    acc.push_str(&line);
                    if let Some(ref hook) = on_stdout_line {
                        hook(line.clone());
                    }
                }
            }
        }
        acc
    });

    let status = child.wait().await?;
    let stderr_acc = stderr_task.await.unwrap_or_default();
    let stdout_acc = stdout_task.await.unwrap_or_default();

    Ok(std::process::Output {
        status,
        stdout: stdout_acc.into_bytes(),
        stderr: stderr_acc.into_bytes(),
    })
}

#[cfg(test)]
mod tests {
    use super::probe_container_runtime_cli;

    #[test]
    fn probe_reports_actionable_error_for_old_client() {
        let err = probe_container_runtime_cli("/nonexistent-docker-xyz").unwrap_err();
        assert!(err.contains("version probe") || err.contains("too old"));
    }
}
