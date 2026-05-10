//! Invoke `docker` / `podman` CLI. Author: kejiqing

use tokio::process::Command;

pub async fn runtime_exec(bin: &str, args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new(bin).args(args).output().await
}
