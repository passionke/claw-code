//! OpenTelemetry OTLP export for self-hosted Langfuse. Author: kejiqing
//!
//! Reads `LANGFUSE_*` + `CLAW_OTEL_*` from the environment; independent of `TelemetrySink` / JSONL.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use opentelemetry::global;
use opentelemetry::propagation::{Extractor, Injector};
use opentelemetry::trace::{TraceContextExt, Tracer};
use opentelemetry::{Context, KeyValue};

pub use opentelemetry::ContextGuard as OtelContextGuard;
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::TracerProvider;
use opentelemetry_sdk::Resource;

const CLAW_OTEL_ENABLED_ENV: &str = "CLAW_OTEL_ENABLED";
const CLAW_OTEL_LOG_PROMPTS_ENV: &str = "CLAW_OTEL_LOG_PROMPTS";
const LANGFUSE_PUBLIC_KEY_ENV: &str = "LANGFUSE_PUBLIC_KEY";
const LANGFUSE_SECRET_KEY_ENV: &str = "LANGFUSE_SECRET_KEY";
const LANGFUSE_BASE_URL_ENV: &str = "LANGFUSE_BASE_URL";
const OTEL_EXPORTER_OTLP_ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
const OTEL_EXPORTER_OTLP_HEADERS_ENV: &str = "OTEL_EXPORTER_OTLP_HEADERS";
const OTEL_SERVICE_NAME_ENV: &str = "OTEL_SERVICE_NAME";
const TRACEPARENT_ENV: &str = "TRACEPARENT";

static TRACER_PROVIDER: OnceLock<Mutex<Option<TracerProvider>>> = OnceLock::new();

struct StringMapCarrier<'a>(pub &'a mut HashMap<String, String>);

impl Injector for StringMapCarrier<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), value);
    }
}

struct HashMapExtractor<'a>(&'a HashMap<String, String>);

impl Extractor for HashMapExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(String::as_str).collect()
    }
}

/// Whether OTEL export is enabled (`CLAW_OTEL_ENABLED` truthy and Langfuse config present).
#[must_use]
pub fn otel_enabled() -> bool {
    if !env_truthy(CLAW_OTEL_ENABLED_ENV) {
        return false;
    }
    resolve_langfuse_otlp_config().is_some()
}

/// `CLAW_OTEL_LOG_PROMPTS` — default **on**; `0` / `false` / `off` disables prompt/completion attrs.
#[must_use]
pub fn log_prompts_enabled() -> bool {
    match std::env::var(CLAW_OTEL_LOG_PROMPTS_ENV) {
        Err(_) => true,
        Ok(value) => {
            let v = value.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "no" | "off")
        }
    }
}

/// Resolve OTLP endpoint + HTTP headers from env.
#[must_use]
pub fn resolve_langfuse_otlp_config() -> Option<(String, HashMap<String, String>)> {
    if let Ok(endpoint) = std::env::var(OTEL_EXPORTER_OTLP_ENDPOINT_ENV) {
        let endpoint = endpoint.trim();
        if !endpoint.is_empty() {
            let headers = parse_otlp_headers(
                &std::env::var(OTEL_EXPORTER_OTLP_HEADERS_ENV).unwrap_or_default(),
            );
            return Some((endpoint.to_string(), headers));
        }
    }

    let public_key = std::env::var(LANGFUSE_PUBLIC_KEY_ENV)
        .ok()
        .map(|s| trim_quotes(s.trim()))
        .filter(|s| !s.is_empty())?;
    let secret_key = std::env::var(LANGFUSE_SECRET_KEY_ENV)
        .ok()
        .map(|s| trim_quotes(s.trim()))
        .filter(|s| !s.is_empty())?;
    let base_url = std::env::var(LANGFUSE_BASE_URL_ENV)
        .ok()
        .map(|s| trim_quotes(s.trim()))
        .filter(|s| !s.is_empty())?;

    let endpoint = format!("{}/api/public/otel", base_url.trim_end_matches('/'));
    let auth_raw = format!("{public_key}:{secret_key}");
    let mut headers = HashMap::new();
    headers.insert(
        String::from("Authorization"),
        format!("Basic {}", STANDARD.encode(auth_raw.as_bytes())),
    );
    Some((endpoint, headers))
}

/// `with_endpoint` uses the URL as-is; Langfuse expects `/api/public/otel/v1/traces`.
fn ensure_otlp_traces_endpoint(endpoint: &str) -> String {
    let base = endpoint.trim().trim_end_matches('/');
    if base.ends_with("/v1/traces") {
        base.to_string()
    } else {
        format!("{base}/v1/traces")
    }
}

fn parse_otlp_headers(raw: &str) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    for part in raw.split(',') {
        let part = part.trim();
        if let Some((name, value)) = part.split_once('=') {
            headers.insert(name.trim().to_string(), value.trim().to_string());
        }
    }
    headers
}

fn trim_quotes(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len().saturating_sub(1)].to_string()
    } else {
        s.to_string()
    }
}

fn env_truthy(key: &str) -> bool {
    std::env::var(key)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn default_service_name() -> String {
    std::env::var(OTEL_SERVICE_NAME_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "claw".to_string())
}

/// Initialize global OTEL tracer provider (`BatchSpanProcessor` + Tokio). No-op when disabled.
pub fn init_otel_from_env() -> bool {
    if !otel_enabled() {
        return false;
    }
    let Some((endpoint, headers)) = resolve_langfuse_otlp_config() else {
        return false;
    };
    let traces_endpoint = ensure_otlp_traces_endpoint(&endpoint);

    let slot = TRACER_PROVIDER.get_or_init(|| Mutex::new(None));
    let Ok(mut guard) = slot.lock() else {
        return false;
    };
    if guard.is_some() {
        return true;
    }

    let service_name = default_service_name();
    let exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(traces_endpoint)
        .with_headers(headers)
        .build()
    {
        Ok(exporter) => exporter,
        Err(e) => {
            eprintln!("telemetry::otel: exporter build failed: {e}");
            return false;
        }
    };

    let provider = TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(Resource::new(vec![KeyValue::new(
            "service.name",
            service_name,
        )]))
        .build();

    global::set_text_map_propagator(TraceContextPropagator::new());
    global::set_tracer_provider(provider.clone());
    *guard = Some(provider);
    true
}

/// Flush and shut down the OTEL exporter.
pub fn shutdown_otel() {
    let Some(slot) = TRACER_PROVIDER.get() else {
        return;
    };
    let Ok(mut guard) = slot.lock() else {
        return;
    };
    if let Some(provider) = guard.take() {
        if let Err(e) = provider.shutdown() {
            eprintln!("telemetry::otel: shutdown failed: {e}");
        }
    }
}

/// Named tracer from the global provider.
#[must_use]
pub fn tracer(instrumentation_name: &'static str) -> opentelemetry::global::BoxedTracer {
    if otel_enabled() {
        let _ = init_otel_from_env();
    }
    opentelemetry::global::tracer(instrumentation_name)
}

/// W3C `traceparent` string for the active context.
#[must_use]
pub fn inject_traceparent(ctx: &Context) -> Option<String> {
    let mut carrier = HashMap::new();
    global::get_text_map_propagator(|prop| {
        prop.inject_context(ctx, &mut StringMapCarrier(&mut carrier));
    });
    carrier.get("traceparent").cloned()
}

/// Build context from W3C `traceparent` (task file or `TRACEPARENT` env).
#[must_use]
pub fn context_from_traceparent(traceparent: &str) -> Context {
    let tp = traceparent.trim();
    if tp.is_empty() {
        return Context::current();
    }
    let carrier = HashMap::from([(String::from("traceparent"), tp.to_string())]);
    global::get_text_map_propagator(|prop| prop.extract(&HashMapExtractor(&carrier)))
}

/// Active context from `TRACEPARENT` env when set.
#[must_use]
pub fn context_from_env_traceparent() -> Context {
    std::env::var(TRACEPARENT_ENV)
        .ok()
        .map_or_else(Context::current, |tp| context_from_traceparent(&tp))
}

/// Repeat Langfuse trace-level attrs on the active span in `cx`.
pub fn set_langfuse_trace_attrs_on_context(
    cx: &Context,
    session_id: &str,
    turn_id: &str,
    request_id: &str,
) {
    let span = cx.span();
    span.set_attribute(KeyValue::new("langfuse.session.id", session_id.to_string()));
    span.set_attribute(KeyValue::new(
        "langfuse.trace.metadata.turn_id",
        turn_id.to_string(),
    ));
    span.set_attribute(KeyValue::new(
        "langfuse.trace.metadata.request_id",
        request_id.to_string(),
    ));
}

/// Start a span, optionally as child of `parent` context.
pub fn start_span_with_parent(
    instrumentation: &'static str,
    name: &'static str,
    parent: Option<&Context>,
) -> Context {
    let tracer = tracer(instrumentation);
    let base = parent.cloned().unwrap_or_else(Context::current);
    let span = tracer.start_with_context(name, &base);
    base.with_span(span)
}

/// Lightweight span handle (`Send`); attach with [`OtelSpanGuard::enter`] for child propagation.
#[derive(Debug)]
pub struct OtelSpanGuard {
    cx: Context,
    finished: std::sync::atomic::AtomicBool,
}

impl OtelSpanGuard {
    #[must_use]
    pub fn start(
        instrumentation: &'static str,
        name: &'static str,
        parent: Option<&Context>,
    ) -> Option<Self> {
        if !otel_enabled() {
            return None;
        }
        Some(Self {
            cx: start_span_with_parent(instrumentation, name, parent),
            finished: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub fn context(&self) -> &Context {
        &self.cx
    }

    pub fn enter(&self) -> opentelemetry::ContextGuard {
        self.cx.clone().attach()
    }

    pub fn set_langfuse_trace_attrs(&self, session_id: &str, turn_id: &str, request_id: &str) {
        set_langfuse_trace_attrs_on_context(&self.cx, session_id, turn_id, request_id);
    }

    pub fn set_attribute(&self, key: &'static str, value: impl Into<String>) {
        self.cx
            .span()
            .set_attribute(KeyValue::new(key, value.into()));
    }

    pub fn set_ok(&self) {
        self.cx.span().set_status(opentelemetry::trace::Status::Ok);
        self.finished
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn set_error(&self, message: impl Into<String>) {
        self.cx
            .span()
            .set_status(opentelemetry::trace::Status::error(message.into()));
        self.finished
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Drop for OtelSpanGuard {
    fn drop(&mut self) {
        if !self.finished.load(std::sync::atomic::Ordering::Relaxed) {
            self.cx.span().set_status(opentelemetry::trace::Status::Ok);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_prompts_enabled_by_default() {
        let _guard = EnvGuard::remove(CLAW_OTEL_LOG_PROMPTS_ENV);
        assert!(log_prompts_enabled());
    }

    #[test]
    fn resolve_endpoint_from_langfuse_base_url() {
        let _g1 = EnvGuard::set(LANGFUSE_PUBLIC_KEY_ENV, Some("pk-test"));
        let _g2 = EnvGuard::set(LANGFUSE_SECRET_KEY_ENV, Some("sk-test"));
        let _g3 = EnvGuard::set(LANGFUSE_BASE_URL_ENV, Some("http://10.22.28.94:8090"));
        let _g4 = EnvGuard::remove(OTEL_EXPORTER_OTLP_ENDPOINT_ENV);
        let (endpoint, headers) = resolve_langfuse_otlp_config().expect("config");
        assert_eq!(endpoint, "http://10.22.28.94:8090/api/public/otel");
        assert!(headers.get("Authorization").unwrap().starts_with("Basic "));
    }

    struct EnvGuard {
        key: &'static str,
        original: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: Option<&str>) -> Self {
            let original = std::env::var_os(key);
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
            Self { key, original }
        }

        fn remove(key: &'static str) -> Self {
            Self::set(key, None)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.original.take() {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
