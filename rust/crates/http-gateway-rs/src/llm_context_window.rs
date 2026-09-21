//! Admin LLM context-window lookup + first-write probe. Author: kejiqing
//!
//! Lookup is best-effort (`ok: false` is still HTTP 200). First write of a length
//! for a `baseUrl`+model ID pair probes the upstream; failure rejects that number
//! only (other card fields still save).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::gateway_global_settings::ActiveLlmRuntime;
use crate::gateway_llm_cluster_store::resolve_llm_cluster_id;
use crate::gateway_llm_model_apply::{
    normalize_model_name_for_upstream, normalize_upstream_base_url,
};
use crate::llm_probe::{self, LlmTestRequest};
use crate::session_db::GatewaySessionDb;

const LOOKUP_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextWindowLookupRequest {
    #[serde(rename = "baseModelUrl")]
    pub base_model_url: String,
    #[serde(rename = "modelName")]
    pub model_name: String,
    #[serde(default, rename = "apiKey")]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextWindowLookupResponse {
    pub ok: bool,
    #[serde(rename = "suggestedTokens", skip_serializing_if = "Option::is_none")]
    pub suggested_tokens: Option<u32>,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextWindowVerifyRequest {
    #[serde(rename = "baseModelUrl")]
    pub base_model_url: String,
    #[serde(rename = "modelName")]
    pub model_name: String,
    #[serde(rename = "contextWindowTokens")]
    pub context_window_tokens: u32,
    #[serde(default, rename = "apiKey")]
    pub api_key: Option<String>,
    #[serde(default, rename = "modelId")]
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextWindowVerifyResponse {
    pub ok: bool,
    #[serde(
        rename = "contextWindowTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub context_window_tokens: Option<u32>,
    #[serde(rename = "skippedProbe", default)]
    pub skipped_probe: bool,
    pub message: String,
}

#[must_use]
pub fn normalize_base_model_url_key(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

#[must_use]
pub fn same_llm_endpoint(
    left_url: &str,
    left_model: &str,
    right_url: &str,
    right_model: &str,
) -> bool {
    normalize_base_model_url_key(left_url) == normalize_base_model_url_key(right_url)
        && left_model.trim() == right_model.trim()
}

#[must_use]
pub fn normalize_positive_window(tokens: Option<u32>) -> Option<u32> {
    tokens.filter(|n| *n > 0)
}

#[must_use]
pub fn window_i32(tokens: Option<u32>) -> Option<i32> {
    normalize_positive_window(tokens).and_then(|n| i32::try_from(n).ok())
}

#[must_use]
pub fn window_u32(tokens: Option<i32>) -> Option<u32> {
    tokens
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
}

/// When `model/info` declared `max_input_tokens`, the user value must be `<=` it.
pub fn check_user_window_against_declared(user: u32, declared: Option<u32>) -> Result<(), String> {
    if user == 0 {
        return Err("contextWindowTokens must be greater than 0".into());
    }
    if let Some(max) = declared {
        if user > max {
            return Err(format!(
                "填写的窗口 {user} 大于网关声明的 max_input_tokens {max}"
            ));
        }
    }
    Ok(())
}

/// Parse LiteLLM-style `{ data: [ { model_name, model_info: { max_input_tokens } } ] }`.
#[must_use]
pub fn parse_max_input_tokens(body: &Value, model_name: &str) -> Option<u32> {
    let needle = model_name.trim();
    if needle.is_empty() {
        return None;
    }
    let data = body.get("data").and_then(Value::as_array)?;
    for item in data {
        if !model_info_item_matches(item, needle) {
            continue;
        }
        let info = item.get("model_info")?;
        if let Some(n) = json_positive_u32(info.get("max_input_tokens")) {
            return Some(n);
        }
        if let Some(n) = json_positive_u32(info.get("max_tokens")) {
            return Some(n);
        }
    }
    None
}

fn model_info_item_matches(item: &Value, needle: &str) -> bool {
    let name = item
        .get("model_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if name == needle {
        return true;
    }
    let litellm = item
        .pointer("/litellm_params/model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    litellm == needle || litellm.ends_with(&format!("/{needle}"))
}

fn json_positive_u32(v: Option<&Value>) -> Option<u32> {
    let n = match v? {
        Value::Number(num) => num.as_u64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    u32::try_from(n).ok().filter(|x| *x > 0)
}

fn model_info_urls(base: &str) -> Vec<String> {
    let b = base.trim().trim_end_matches('/');
    let mut urls = vec![format!("{b}/model/info"), format!("{b}/v1/model/info")];
    if let Some(stripped) = b.strip_suffix("/v1") {
        let origin = stripped.trim_end_matches('/');
        urls.push(format!("{origin}/model/info"));
        urls.push(format!("{origin}/v1/model/info"));
    }
    urls.sort();
    urls.dedup();
    urls
}

async fn fetch_model_info_json(base: &str, api_key: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(LOOKUP_TIMEOUT)
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let mut last_err = "model/info unreachable".to_string();
    for url in model_info_urls(base) {
        let mut req = client.get(&url);
        let key = api_key.trim();
        if !key.is_empty() {
            req = req.bearer_auth(key);
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.as_u16() == 404 {
                    last_err = format!("{url} returned 404");
                    continue;
                }
                let body = resp.text().await.map_err(|e| format!("read {url}: {e}"))?;
                if !status.is_success() {
                    last_err = format!("{url} returned {status}");
                    continue;
                }
                return serde_json::from_str(&body).map_err(|e| format!("{url} is not JSON: {e}"));
            }
            Err(e) => {
                last_err = format!("{url}: {e}");
            }
        }
    }
    Err(last_err)
}

/// Side-channel hint only. Failures become `ok: false`; never an HTTP error.
pub async fn lookup_context_window(req: ContextWindowLookupRequest) -> ContextWindowLookupResponse {
    let Some(base) = normalize_upstream_base_url(&req.base_model_url) else {
        return ContextWindowLookupResponse {
            ok: false,
            suggested_tokens: None,
            message: "Base URL 无效，可手动填写窗口".into(),
        };
    };
    let Some(model) = normalize_model_name_for_upstream(&req.model_name, &base) else {
        return ContextWindowLookupResponse {
            ok: false,
            suggested_tokens: None,
            message: "模型 ID 无效，可手动填写窗口".into(),
        };
    };
    let api_key = req.api_key.unwrap_or_default();
    match fetch_model_info_json(&base, &api_key).await {
        Ok(body) => match parse_max_input_tokens(&body, &model) {
            Some(tokens) => ContextWindowLookupResponse {
                ok: true,
                suggested_tokens: Some(tokens),
                message: format!("网关建议 max_input_tokens = {tokens}"),
            },
            None => ContextWindowLookupResponse {
                ok: false,
                suggested_tokens: None,
                message: "网关未列出该模型的 max_input_tokens，可手动填写".into(),
            },
        },
        Err(err) => ContextWindowLookupResponse {
            ok: false,
            suggested_tokens: None,
            message: format!("未查到建议值（{err}），可手动填写"),
        },
    }
}

fn draft_runtime(base: String, model: String, api_key: String) -> ActiveLlmRuntime {
    ActiveLlmRuntime {
        model_id: "draft".into(),
        model_rev: String::new(),
        base_model_url: base,
        model_name: model,
        api_key,
        supports_vision: false,
        supports_video: false,
        supports_audio: false,
        applied_at_ms: None,
        context_window_tokens: None,
    }
}

async fn resolve_api_key_for_verify(
    db: &GatewaySessionDb,
    req: &ContextWindowVerifyRequest,
    proj_id: Option<i64>,
) -> Result<String, String> {
    if let Some(key) = req
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok(key.to_string());
    }
    let Some(model_id) = req
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Err("探测需要 API Key（表单填写，或保存过的模型卡）".into());
    };
    let runtime = if let Some(proj_id) = proj_id {
        crate::gateway_project_llm::load_llm_runtime_for_project_model_id(db, proj_id, model_id)
            .await?
    } else {
        crate::gateway_global_settings::load_llm_runtime_for_model_id(db, model_id).await?
    };
    if runtime.api_key.trim().is_empty() {
        return Err("apiKey is not configured".into());
    }
    Ok(runtime.api_key)
}

pub async fn verify_window_against_upstream(
    base_url: &str,
    model_name: &str,
    api_key: &str,
    user_tokens: u32,
) -> Result<(), String> {
    check_user_window_against_declared(user_tokens, None)?;
    let base =
        normalize_upstream_base_url(base_url).ok_or_else(|| "invalid baseModelUrl".to_string())?;
    let model = normalize_model_name_for_upstream(model_name, &base)
        .ok_or_else(|| "invalid modelName".to_string())?;
    let declared = match fetch_model_info_json(&base, api_key).await {
        Ok(body) => parse_max_input_tokens(&body, &model),
        Err(_) => None,
    };
    check_user_window_against_declared(user_tokens, declared)?;
    let runtime = draft_runtime(base, model, api_key.to_string());
    let probe = llm_probe::probe_with_runtime(
        runtime,
        LlmTestRequest {
            model_id: "draft".into(),
            prompt: Some("ping".into()),
            thinking_enabled: None,
            temperature: None,
            top_p: None,
            max_tokens: Some(16),
            frequency_penalty: None,
            presence_penalty: None,
            reasoning_effort: None,
        },
    )
    .await?;
    if probe.ok {
        Ok(())
    } else {
        let detail = probe.errors.join("; ");
        Err(if detail.is_empty() {
            "模型探测失败，拒绝写入该窗口数字".into()
        } else {
            detail
        })
    }
}

/// First write of a length for this URL+ID: probe; otherwise persist the number as-is.
pub async fn resolve_window_for_save(
    db: &GatewaySessionDb,
    base_url: &str,
    model_name: &str,
    api_key: &str,
    requested: Option<u32>,
) -> (Option<u32>, Option<String>) {
    let Some(tokens) = normalize_positive_window(requested) else {
        return (None, None);
    };
    let Some(cluster_id) = resolve_llm_cluster_id() else {
        return (None, Some("CLAW_CLUSTER_ID is not set".into()));
    };
    match db
        .find_saved_context_window(&cluster_id, base_url, model_name)
        .await
    {
        Ok(Some(_)) => (Some(tokens), None),
        Ok(None) => {
            match verify_window_against_upstream(base_url, model_name, api_key, tokens).await {
                Ok(()) => (Some(tokens), None),
                Err(reason) => (None, Some(reason)),
            }
        }
        Err(e) => (None, Some(format!("lookup saved window: {e}"))),
    }
}

pub async fn verify_context_window(
    db: &GatewaySessionDb,
    req: ContextWindowVerifyRequest,
    proj_id: Option<i64>,
) -> ContextWindowVerifyResponse {
    let tokens = req.context_window_tokens;
    if tokens == 0 {
        return ContextWindowVerifyResponse {
            ok: false,
            context_window_tokens: None,
            skipped_probe: false,
            message: "窗口必须大于 0".into(),
        };
    }
    let Some(cluster_id) = resolve_llm_cluster_id() else {
        return ContextWindowVerifyResponse {
            ok: false,
            context_window_tokens: None,
            skipped_probe: false,
            message: "CLAW_CLUSTER_ID is not set".into(),
        };
    };
    match db
        .find_saved_context_window(&cluster_id, &req.base_model_url, &req.model_name)
        .await
    {
        Ok(Some(_)) => {
            return ContextWindowVerifyResponse {
                ok: true,
                context_window_tokens: Some(tokens),
                skipped_probe: true,
                message: "系统已有同一 Base URL + 模型 ID 的窗口，跳过探测".into(),
            };
        }
        Ok(None) => {}
        Err(e) => {
            return ContextWindowVerifyResponse {
                ok: false,
                context_window_tokens: None,
                skipped_probe: false,
                message: format!("查询已存窗口失败: {e}"),
            };
        }
    }
    let api_key = match resolve_api_key_for_verify(db, &req, proj_id).await {
        Ok(k) => k,
        Err(message) => {
            return ContextWindowVerifyResponse {
                ok: false,
                context_window_tokens: None,
                skipped_probe: false,
                message,
            };
        }
    };
    match verify_window_against_upstream(&req.base_model_url, &req.model_name, &api_key, tokens)
        .await
    {
        Ok(()) => ContextWindowVerifyResponse {
            ok: true,
            context_window_tokens: Some(tokens),
            skipped_probe: false,
            message: "探测通过，可以写入该窗口".into(),
        },
        Err(message) => ContextWindowVerifyResponse {
            ok: false,
            context_window_tokens: None,
            skipped_probe: false,
            message,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn url_key_strips_trailing_slash() {
        assert_eq!(
            normalize_base_model_url_key("https://api.example.com/v1/"),
            "https://api.example.com/v1"
        );
        assert!(same_llm_endpoint(
            "https://api.example.com/v1/",
            "qwen3.8-max-0902",
            "https://api.example.com/v1",
            "qwen3.8-max-0902",
        ));
        assert!(!same_llm_endpoint(
            "https://api.example.com/v1",
            "qwen3.8-max-0902",
            "https://other.example.com/v1",
            "qwen3.8-max-0902",
        ));
    }

    #[test]
    fn parse_litellm_max_input_tokens_for_matching_model() {
        let body = json!({
            "data": [{
                "model_name": "qwen3.8-max-0902",
                "litellm_params": { "model": "dashscope/qwen3.8-max-0902" },
                "model_info": { "max_input_tokens": 991808, "max_tokens": 1000000 }
            }]
        });
        assert_eq!(
            parse_max_input_tokens(&body, "qwen3.8-max-0902"),
            Some(991_808)
        );
        assert_eq!(parse_max_input_tokens(&body, "other-model"), None);
    }

    #[test]
    fn user_window_must_not_exceed_declared_max() {
        assert!(check_user_window_against_declared(991_808, Some(991_808)).is_ok());
        let err = check_user_window_against_declared(991_809, Some(991_808)).unwrap_err();
        assert!(err.contains("991809"));
        assert!(check_user_window_against_declared(1000, None).is_ok());
        assert!(check_user_window_against_declared(0, None).is_err());
    }

    #[test]
    fn lookup_invalid_url_is_ok_false_not_an_error_path() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let resp = rt.block_on(lookup_context_window(ContextWindowLookupRequest {
            base_model_url: "not-a-url".into(),
            model_name: "qwen".into(),
            api_key: None,
        }));
        assert!(!resp.ok);
        assert!(resp.suggested_tokens.is_none());
        assert!(!resp.message.is_empty());
    }
}
