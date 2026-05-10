//! Gateway tracing: stdout + optional **daily JSON files** under a dedicated directory.
//! When file logging is enabled, **stdout is also JSON** so both layers share the same `fmt` shape.
//! Author: kejiqing

use std::path::{Path, PathBuf};

use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Directory for daily-rotated `http-gateway.*.log` JSON lines. `None` = file sink disabled.
pub fn resolved_file_log_dir(work_root: &Path) -> Option<PathBuf> {
    let disable = matches!(
        std::env::var("CLAW_GATEWAY_FILE_LOG")
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref(),
        Ok("0" | "false" | "no" | "off")
    );
    if disable {
        return None;
    }
    std::env::var("CLAW_GATEWAY_LOG_DIR")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(work_root.join(".claw-gateway-logs")))
}

fn env_filter() -> EnvFilter {
    if let Ok(level) = std::env::var("CLAW_LOG_LEVEL") {
        EnvFilter::new(level)
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    }
}

fn log_format_json() -> bool {
    std::env::var("CLAW_LOG_FORMAT")
        .unwrap_or_else(|_| "json".to_string())
        .trim()
        .eq_ignore_ascii_case("json")
}

/// Call **after** `CLAW_WORK_ROOT` is known. Installs global subscriber (stdout + optional file).
pub fn init(work_root: &Path) {
    let filter = env_filter();
    let file_dir = resolved_file_log_dir(work_root);

    if let Some(dir) = file_dir {
        if std::fs::create_dir_all(&dir).is_err() {
            eprintln!(
                "http-gateway-rs: cannot create CLAW_GATEWAY_LOG_DIR {}",
                dir.display()
            );
            init_stdout_only(&filter);
            return;
        }
        let appender = tracing_appender::rolling::daily(&dir, "http-gateway");
        let (writer, guard) = tracing_appender::non_blocking(appender);
        #[allow(clippy::mem_forget)]
        {
            std::mem::forget(guard);
        }
        let file_layer = fmt::layer()
            .json()
            .with_writer(writer)
            .with_target(true)
            .with_current_span(true);
        let stdout_layer = fmt::layer()
            .json()
            .with_writer(std::io::stdout)
            .with_target(true)
            .with_current_span(false);
        tracing_subscriber::registry()
            .with(filter)
            .with(stdout_layer)
            .with(file_layer)
            .init();
        return;
    }

    init_stdout_only(&filter);
}

fn init_stdout_only(filter: &EnvFilter) {
    if log_format_json() {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter.clone())
            .with_current_span(false)
            .with_target(true)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter.clone())
            .with_target(true)
            .init();
    }
}
