//! Derive claude-tap `--tap-client` from active LLM `baseModelUrl` path. Author: kejiqing

/// Default when URL has no Messages/Responses endpoint suffix (Admin `…/v1`).
pub const DEFAULT_TAP_CLIENT: &str = "openai";

/// Map `baseModelUrl` path → `CLAW_TAP_CLIENT` / `--tap-client` value.
///
/// Longer suffixes win: `/chat/completions` before bare segments. Host is ignored.
#[must_use]
pub fn tap_client_from_base_model_url(raw: &str) -> &'static str {
    let path = normalize_url_path(raw);
    if path_ends_with(&path, "/chat/completions") {
        return "openai";
    }
    if path_ends_with(&path, "/messages") {
        return "claude";
    }
    if path_ends_with(&path, "/responses") {
        return "codex";
    }
    DEFAULT_TAP_CLIENT
}

/// True when apply should recreate observe (wire protocol / tap client changed).
#[must_use]
pub fn tap_client_changed(old_url: &str, new_url: &str) -> bool {
    tap_client_from_base_model_url(old_url) != tap_client_from_base_model_url(new_url)
}

fn normalize_url_path(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }
    let after_scheme = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .or_else(|| s.strip_prefix("HTTPS://"))
        .or_else(|| s.strip_prefix("HTTP://"))
        .unwrap_or(s);
    let path_and_more = match after_scheme.find('/') {
        Some(i) => &after_scheme[i..],
        None => return String::new(),
    };
    let path_only = path_and_more
        .split(['?', '#'])
        .next()
        .unwrap_or(path_and_more);
    let lower = path_only.to_ascii_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut prev_slash = false;
    for ch in lower.chars() {
        if ch == '/' {
            if prev_slash {
                continue;
            }
            prev_slash = true;
            out.push('/');
        } else {
            prev_slash = false;
            out.push(ch);
        }
    }
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    out
}

fn path_ends_with(path: &str, suffix: &str) -> bool {
    path == suffix || path.ends_with(suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_path_is_claude() {
        assert_eq!(
            tap_client_from_base_model_url("https://api.anthropic.com/v1/messages"),
            "claude"
        );
    }

    #[test]
    fn responses_path_is_codex() {
        assert_eq!(
            tap_client_from_base_model_url("https://api.openai.com/v1/responses"),
            "codex"
        );
    }

    #[test]
    fn chat_completions_path_is_openai() {
        assert_eq!(
            tap_client_from_base_model_url("https://api.openai.com/v1/chat/completions"),
            "openai"
        );
    }

    #[test]
    fn bare_v1_is_openai() {
        assert_eq!(
            tap_client_from_base_model_url("https://api.deepseek.com/v1"),
            "openai"
        );
    }

    #[test]
    fn trailing_slash_compatible_mode_v1_is_openai() {
        assert_eq!(
            tap_client_from_base_model_url("https://dashscope.aliyuncs.com/compatible-mode/v1/"),
            "openai"
        );
    }

    #[test]
    fn case_and_extra_slashes_normalize() {
        assert_eq!(
            tap_client_from_base_model_url("https://api.anthropic.com/v1/Messages/"),
            "claude"
        );
        assert_eq!(
            tap_client_from_base_model_url("https://api.openai.com/v1//responses"),
            "codex"
        );
    }

    #[test]
    fn empty_or_invalid_defaults_openai() {
        assert_eq!(tap_client_from_base_model_url(""), DEFAULT_TAP_CLIENT);
        assert_eq!(
            tap_client_from_base_model_url("not-a-url"),
            DEFAULT_TAP_CLIENT
        );
        assert_eq!(
            tap_client_from_base_model_url("https://host.only"),
            DEFAULT_TAP_CLIENT
        );
    }

    #[test]
    fn chat_completions_not_confused_with_other_suffixes() {
        assert_eq!(
            tap_client_from_base_model_url("https://x.example/v1/chat/completions"),
            "openai"
        );
        assert_ne!(
            tap_client_from_base_model_url("https://x.example/v1/chat/completions"),
            "claude"
        );
    }

    #[test]
    fn tap_client_changed_same_protocol_different_host() {
        assert!(!tap_client_changed(
            "https://api.deepseek.com/v1",
            "https://other.example.com/v1"
        ));
    }

    #[test]
    fn tap_client_changed_v1_to_messages() {
        assert!(tap_client_changed(
            "https://api.deepseek.com/v1",
            "https://api.anthropic.com/v1/messages"
        ));
    }

    #[test]
    fn tap_client_changed_messages_to_responses() {
        assert!(tap_client_changed(
            "https://api.anthropic.com/v1/messages",
            "https://api.openai.com/v1/responses"
        ));
    }

    #[test]
    fn tap_client_changed_chat_completions_to_bare_v1() {
        assert!(!tap_client_changed(
            "https://api.openai.com/v1/chat/completions",
            "https://api.openai.com/v1"
        ));
    }
}
