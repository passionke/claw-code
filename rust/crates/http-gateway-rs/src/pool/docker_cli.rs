//! Invoke `docker` / `podman` CLI. Author: kejiqing

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

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

/// `docker exec` (long-running): stream stderr lines to tracing while collecting stdout/stderr.
/// Without this, progress and errors inside the worker only appear after the process exits
/// because `output()` buffers until EOF.
fn argv_summary(args: &[&str], max_bytes: usize) -> String {
    let s = args.join(" ");
    if s.len() <= max_bytes {
        return s;
    }
    let start = s.floor_char_boundary(s.len().saturating_sub(max_bytes));
    format!("…{}", &s[start..])
}

pub async fn runtime_exec_with_live_stderr(
    bin: &str,
    args: &[&str],
    request_id: Option<&str>,
) -> std::io::Result<std::process::Output> {
    tracing::debug!(
        target: "claw_gateway_pool",
        component = "docker_cli",
        phase = "exec_spawn",
        %bin,
        argv_summary = %argv_summary(args, 1800),
        request_id = request_id.unwrap_or(""),
        "spawning docker/podman exec (stderr streamed to target claw_gateway_solve)"
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

    let mut stdout_buf = Vec::new();
    {
        let mut reader = BufReader::new(stdout);
        reader.read_to_end(&mut stdout_buf).await?;
    }

    let status = child.wait().await?;
    let stderr_acc = stderr_task.await.unwrap_or_default();

    Ok(std::process::Output {
        status,
        stdout: stdout_buf,
        stderr: stderr_acc.into_bytes(),
    })
}
