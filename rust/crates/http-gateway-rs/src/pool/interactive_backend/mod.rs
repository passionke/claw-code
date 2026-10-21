//! e2b runtime helpers historically named "interactive" (NAS / Observe / worker tap).
//! REPL lease API (OVS / web CLI) removed. Author: kejiqing

mod e2b_nas_api_singleton;
mod e2b_worker_tap;

pub use e2b_nas_api_singleton::E2bNasApiSingleton;
pub use e2b_worker_tap::{
    e2b_worker_llm_env, e2b_worker_solve_route, load_e2b_observe_proxy_base_url,
    E2B_WORKER_TAP_PLACEHOLDER_API_KEY,
};

/// Interactive backend is always e2b (legacy `CLAW_INTERACTIVE_BACKEND` removed).
#[must_use]
pub fn interactive_backend_is_e2b() -> bool {
    if let Some(raw) = std::env::var("CLAW_INTERACTIVE_BACKEND").ok() {
        let v = raw.trim();
        if !v.is_empty()
            && !v.eq_ignore_ascii_case("e2b")
            && !v.eq_ignore_ascii_case("fc")
            && !v.eq_ignore_ascii_case("firecracker")
        {
            eprintln!(
                "http-gateway-rs: CLAW_INTERACTIVE_BACKEND={v:?} removed; delete from .env (e2b-only)"
            );
            std::process::exit(1);
        }
    }
    true
}

/// True when e2b session-observe singleton should run (Admin Live on e2b).
#[must_use]
pub fn e2b_observe_is_enabled() -> bool {
    interactive_backend_is_e2b()
        && !matches!(
            std::env::var("CLAW_E2B_OBSERVE")
                .ok()
                .map(|v| v.trim().to_ascii_lowercase())
                .as_deref(),
            Some("0" | "false" | "no" | "off")
        )
}
