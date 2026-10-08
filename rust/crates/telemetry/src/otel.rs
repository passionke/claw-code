//! OpenTelemetry OTLP HTTP export (SkyWalking / any OTLP backend). Author: kejiqing
//!
//! Reads `CLAW_OTEL_*` + `OTEL_EXPORTER_OTLP_*` from the environment; independent of `TelemetrySink` / JSONL.

use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::future::BoxFuture;
use opentelemetry::global;
use opentelemetry::propagation::{Extractor, Injector};
use opentelemetry::trace::{
    Span, SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState, Tracer,
};
use opentelemetry::{Context, KeyValue};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::export::trace::{ExportResult, SpanData, SpanExporter};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::TracerProvider;
use opentelemetry_sdk::Resource;

pub use opentelemetry::ContextGuard as OtelContextGuard;

const CLAW_OTEL_ENABLED_ENV: &str = "CLAW_OTEL_ENABLED";
const CLAW_OTEL_LOG_PROMPTS_ENV: &str = "CLAW_OTEL_LOG_PROMPTS";
const OTEL_EXPORTER_OTLP_ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
const OTEL_EXPORTER_OTLP_HEADERS_ENV: &str = "OTEL_EXPORTER_OTLP_HEADERS";
const OTEL_SERVICE_NAME_ENV: &str = "OTEL_SERVICE_NAME";
const TRACEPARENT_ENV: &str = "TRACEPARENT";
const EXPORT_FAIL_LOG_INTERVAL_MS: u64 = 10_000;

static TRACER_PROVIDER: OnceLock<Mutex<Option<TracerProvider>>> = OnceLock::new();
static LAST_EXPORT_FAIL_LOG_MS: AtomicU64 = AtomicU64::new(0);
static LAST_EMIT_FAIL_LOG_MS: AtomicU64 = AtomicU64::new(0);

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

/// Whether OTEL export is enabled (`CLAW_OTEL_ENABLED` + `OTEL_EXPORTER_OTLP_ENDPOINT`).
#[must_use]
pub fn otel_enabled() -> bool {
    if !env_truthy(CLAW_OTEL_ENABLED_ENV) {
        return false;
    }
    resolve_otlp_config().is_some()
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

/// Resolve OTLP endpoint + HTTP headers from env. Author: kejiqing
#[must_use]
pub fn resolve_otlp_config() -> Option<(String, HashMap<String, String>)> {
    let endpoint = std::env::var(OTEL_EXPORTER_OTLP_ENDPOINT_ENV)
        .ok()
        .map(|s| trim_quotes(s.trim()))
        .filter(|s| !s.is_empty())?;
    let headers = parse_otlp_headers(
        &std::env::var(OTEL_EXPORTER_OTLP_HEADERS_ENV).unwrap_or_default(),
    );
    Some((endpoint, headers))
}

/// `with_endpoint` uses the URL as-is; append `/v1/traces` when missing.
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn endpoint_host_for_log(endpoint: &str) -> String {
    let s = endpoint.trim();
    let without_scheme = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .to_string()
}

/// Rate-limited structured stderr log for OTLP export failures. Author: kejiqing
#[must_use]
pub fn should_log_export_failure(now: u64, interval_ms: u64) -> bool {
    let prev = LAST_EXPORT_FAIL_LOG_MS.load(Ordering::Relaxed);
    if prev == 0 || now.saturating_sub(prev) >= interval_ms {
        LAST_EXPORT_FAIL_LOG_MS.store(now, Ordering::Relaxed);
        true
    } else {
        false
    }
}

/// Structured stderr log for local emit failures (rate-limited). Author: kejiqing
#[allow(dead_code)]
pub fn log_emit_failed(kind: &str, error: &str) {
    let ts = now_ms();
    let prev = LAST_EMIT_FAIL_LOG_MS.load(Ordering::Relaxed);
    if prev != 0 && ts.saturating_sub(prev) < EXPORT_FAIL_LOG_INTERVAL_MS {
        return;
    }
    LAST_EMIT_FAIL_LOG_MS.store(ts, Ordering::Relaxed);
    eprintln!(
        "{}",
        serde_json::json!({
            "event": "telemetry.otel.emit_failed",
            "ts_ms": ts,
            "kind": kind,
            "error": error,
        })
    );
}

/// Thin wrapper: log export failures to stderr (rate-limited), never used on business path. Author: kejiqing
#[derive(Debug)]
struct LoggingSpanExporter<E> {
    inner: E,
    endpoint_host: String,
}

impl<E: SpanExporter> SpanExporter for LoggingSpanExporter<E> {
    fn export(&mut self, batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
        let n = batch.len();
        let host = self.endpoint_host.clone();
        let fut = self.inner.export(batch);
        Box::pin(async move {
            match fut.await {
                Ok(()) => Ok(()),
                Err(e) => {
                    let msg = e.to_string();
                    let ts = now_ms();
                    if should_log_export_failure(ts, EXPORT_FAIL_LOG_INTERVAL_MS) {
                        eprintln!(
                            "{}",
                            serde_json::json!({
                                "event": "telemetry.otel.export_failed",
                                "ts_ms": ts,
                                "error": msg,
                                "dropped_spans": n,
                                "endpoint_host": host,
                            })
                        );
                    }
                    Err(e)
                }
            }
        })
    }

    fn shutdown(&mut self) {
        self.inner.shutdown();
    }

    fn force_flush(&mut self) -> BoxFuture<'static, ExportResult> {
        let host = self.endpoint_host.clone();
        let fut = self.inner.force_flush();
        Box::pin(async move {
            match fut.await {
                Ok(()) => Ok(()),
                Err(e) => {
                    let msg = e.to_string();
                    let ts = now_ms();
                    if should_log_export_failure(ts, EXPORT_FAIL_LOG_INTERVAL_MS) {
                        eprintln!(
                            "{}",
                            serde_json::json!({
                                "event": "telemetry.otel.export_failed",
                                "ts_ms": ts,
                                "error": msg,
                                "dropped_spans": 0,
                                "endpoint_host": host,
                            })
                        );
                    }
                    Err(e)
                }
            }
        })
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

/// Initialize global OTEL tracer provider (`BatchSpanProcessor` + Tokio). No-op when disabled.
pub fn init_otel_from_env() -> bool {
    if !otel_enabled() {
        return false;
    }
    let Some((endpoint, headers)) = resolve_otlp_config() else {
        return false;
    };
    let traces_endpoint = ensure_otlp_traces_endpoint(&endpoint);
    let endpoint_host = endpoint_host_for_log(&endpoint);

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
        Ok(exporter) => LoggingSpanExporter {
            inner: exporter,
            endpoint_host,
        },
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

/// Normalize to W3C 32-lowercase-hex trace id; reject empty / wrong length. Author: kejiqing
fn normalize_w3c_trace_id(raw: &str) -> Option<String> {
    let s = raw.trim().to_ascii_lowercase();
    if s.len() != 32 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    if s.chars().all(|c| c == '0') {
        return None;
    }
    Some(s)
}

/// Remote parent context pinned to `trace_id` (synthetic span id). Author: kejiqing
#[must_use]
pub fn context_from_trace_id(trace_id: &str) -> Option<Context> {
    let tid_hex = normalize_w3c_trace_id(trace_id)?;
    let tid = TraceId::from_hex(&tid_hex).ok()?;
    // Non-zero synthetic remote parent so SpanContext is valid. Author: kejiqing
    let sid = SpanId::from_hex("0100000000000001").ok()?;
    let sc = SpanContext::new(tid, sid, TraceFlags::SAMPLED, true, TraceState::NONE);
    if !sc.is_valid() {
        return None;
    }
    Some(Context::new().with_remote_span_context(sc))
}

/// Parse W3C `traceparent` without global propagator (pre-init safe). Author: kejiqing
fn context_from_w3c_traceparent_raw(tp: &str) -> Option<Context> {
    let parts: Vec<&str> = tp.trim().split('-').collect();
    if parts.len() < 4 || parts[0] != "00" {
        return None;
    }
    let tid_hex = normalize_w3c_trace_id(parts[1])?;
    let sid = parts[2].trim().to_ascii_lowercase();
    if sid.len() != 16
        || !sid.chars().all(|c| c.is_ascii_hexdigit())
        || sid.chars().all(|c| c == '0')
    {
        return None;
    }
    let tid = TraceId::from_hex(&tid_hex).ok()?;
    let span_id = SpanId::from_hex(&sid).ok()?;
    let flags = if parts[3].trim().ends_with('1') {
        TraceFlags::SAMPLED
    } else {
        TraceFlags::default()
    };
    let sc = SpanContext::new(tid, span_id, flags, true, TraceState::NONE);
    if !sc.is_valid() {
        return None;
    }
    Some(Context::new().with_remote_span_context(sc))
}

/// Prefer inbound W3C `traceparent`; else seed from request `trace_id`. Author: kejiqing
#[must_use]
pub fn parent_context_for_inbound(
    inbound_traceparent: Option<&str>,
    trace_id: &str,
) -> Option<Context> {
    if let Some(tp) = inbound_traceparent.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(cx) = context_from_w3c_traceparent_raw(tp) {
            return Some(cx);
        }
    }
    context_from_trace_id(trace_id)
}

/// Neutral trace-level attrs on the active span in `cx`. Author: kejiqing
pub fn set_trace_attrs_on_context(
    cx: &Context,
    session_id: &str,
    turn_id: &str,
    request_id: &str,
) {
    let span = cx.span();
    span.set_attribute(KeyValue::new("session_id", session_id.to_string()));
    span.set_attribute(KeyValue::new("turn_id", turn_id.to_string()));
    span.set_attribute(KeyValue::new("request_id", request_id.to_string()));
}

/// Start a span as child of `parent`, or of `Context::current()` when `parent` is `None`.
///
/// Prefer **explicit** `parent` on async / multi-await paths (e.g. gateway solve).
/// Using `None` is only correct when the caller has already `enter()`'d the intended parent
/// (worker turn) or is inside a sync short-enter block. Author: kejiqing
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

/// Emit a short-lived child span under `Context::current()` (no-op when OTEL off). Author: kejiqing
///
/// Ambient parent only: caller must have set current context (worker `SolveTurnOtelGuard::enter`,
/// or gateway sync short-`enter` around this call). Do not call from an async gap where no one
/// has entered the intended parent — that silently orphans the span, not a business error.
pub fn emit_child_span(name: &str, attrs: &[(&str, String)]) {
    if !otel_enabled() {
        return;
    }
    // Batch exporter requires Tokio; never panic outside a runtime. Author: kejiqing
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let _ = init_otel_from_env();
    let span_name = name.to_string();
    let tracer = opentelemetry::global::tracer("claw-timing");
    let cx = Context::current();
    let mut span = tracer.start_with_context(span_name, &cx);
    for (k, v) in attrs {
        span.set_attribute(KeyValue::new((*k).to_string(), v.clone()));
    }
    span.set_status(opentelemetry::trace::Status::Ok);
    drop(span);
}

/// Lightweight span handle (`Send`); attach with [`OtelSpanGuard::enter`] for child propagation.
///
/// `enter()` returns `OtelContextGuard` (`!Send`) — hold only in sync scopes or within a single
/// task that never moves the guard across `.await` on a `Send` future. Prefer passing
/// `Some(parent)` into [`Self::start`] instead of long-lived ambient enter on async gateway paths.
#[derive(Debug)]
pub struct OtelSpanGuard {
    cx: Context,
    finished: std::sync::atomic::AtomicBool,
}

impl OtelSpanGuard {
    /// `parent = None` ⇒ use `Context::current()` (ambient). Prefer `Some(parent)` when known. Author: kejiqing
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

    pub fn set_trace_attrs(&self, session_id: &str, turn_id: &str, request_id: &str) {
        set_trace_attrs_on_context(&self.cx, session_id, turn_id, request_id);
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
    fn resolve_otlp_from_endpoint_env() {
        let _g1 = EnvGuard::set(
            OTEL_EXPORTER_OTLP_ENDPOINT_ENV,
            Some("http://10.22.28.239:12800"),
        );
        let _g2 = EnvGuard::set(OTEL_EXPORTER_OTLP_HEADERS_ENV, Some("a=b"));
        let (endpoint, headers) = resolve_otlp_config().expect("config");
        assert_eq!(endpoint, "http://10.22.28.239:12800");
        assert_eq!(headers.get("a").map(String::as_str), Some("b"));
    }

    #[test]
    fn otel_enabled_requires_endpoint() {
        let _g1 = EnvGuard::set(CLAW_OTEL_ENABLED_ENV, Some("1"));
        let _g2 = EnvGuard::remove(OTEL_EXPORTER_OTLP_ENDPOINT_ENV);
        assert!(!otel_enabled());
        let _g3 = EnvGuard::set(
            OTEL_EXPORTER_OTLP_ENDPOINT_ENV,
            Some("http://127.0.0.1:12800"),
        );
        assert!(otel_enabled());
    }

    #[test]
    fn emit_child_span_noop_when_disabled() {
        let _g1 = EnvGuard::remove(CLAW_OTEL_ENABLED_ENV);
        let _g2 = EnvGuard::remove(OTEL_EXPORTER_OTLP_ENDPOINT_ENV);
        emit_child_span("timing.test", &[("k", "v".to_string())]);
    }

    #[test]
    fn should_log_export_failure_rate_limits() {
        LAST_EXPORT_FAIL_LOG_MS.store(0, Ordering::Relaxed);
        assert!(should_log_export_failure(1000, 10_000));
        assert!(!should_log_export_failure(2000, 10_000));
        assert!(should_log_export_failure(12_000, 10_000));
    }

    #[test]
    fn context_from_trace_id_pins_w3c_id() {
        let tid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let cx = context_from_trace_id(tid).expect("context");
        assert_eq!(format!("{:032x}", cx.span().span_context().trace_id()), tid);
        assert!(cx.span().span_context().is_remote());
    }

    #[test]
    fn parent_context_prefers_valid_traceparent() {
        let tid = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let parent_span = "cccccccccccccccc";
        let tp = format!("00-{tid}-{parent_span}-01");
        let cx = parent_context_for_inbound(Some(&tp), "dddddddddddddddddddddddddddddddd")
            .expect("context");
        let span = cx.span();
        let sc = span.span_context();
        assert_eq!(format!("{:032x}", sc.trace_id()), tid);
        assert_eq!(format!("{:016x}", sc.span_id()), parent_span);
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
