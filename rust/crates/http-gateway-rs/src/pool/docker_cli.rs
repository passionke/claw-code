//! Container runtime CLI helpers (session tree chown fallback). Author: kejiqing

use std::process::Stdio;
use tokio::process::Command;

/// Run `runtime_bin` with args and extra env; capture stdout/stderr. Author: kejiqing
pub async fn runtime_exec_with_env(
    bin: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> std::io::Result<std::process::Output> {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.output().await
}
