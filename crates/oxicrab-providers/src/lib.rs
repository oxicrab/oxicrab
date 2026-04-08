//! LLM provider implementations for the oxicrab framework.
//!
//! This crate contains all provider-specific code: Anthropic, OpenAI, Gemini,
//! circuit breaker, fallback, and prompt-guided wrappers.

pub mod anthropic;
pub mod anthropic_common;
pub mod anthropic_oauth;
pub mod circuit_breaker;
pub mod errors;
pub mod fallback;
pub mod gemini;
pub mod openai;
pub mod prompt_guided;
pub mod strategy;
mod utils;

use anyhow::Result;
use reqwest::Client;
use serde_json::Value;
use std::sync::LazyLock;
use std::time::Duration;
use tracing::{info, warn};

/// Connect timeout for LLM provider HTTP clients (seconds).
pub const PROVIDER_CONNECT_TIMEOUT_SECS: u64 = 30;
/// Overall request timeout for LLM provider HTTP clients (seconds).
pub const PROVIDER_REQUEST_TIMEOUT_SECS: u64 = 120;

/// Default API URL for Anthropic.
pub(crate) const API_URL_ANTHROPIC: &str = "https://api.anthropic.com/v1/messages";
/// Default API URL for OpenAI.
pub(crate) const API_URL_OPENAI: &str = "https://api.openai.com/v1/chat/completions";
/// Default base URL for Gemini.
pub(crate) const BASE_URL_GEMINI: &str = "https://generativelanguage.googleapis.com/v1";

/// Per-process session affinity ID. Load balancers can use this to route
/// requests from the same process to the same backend for prompt cache locality.
static SESSION_AFFINITY_ID: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());

/// Build a `reqwest::Client` with standard provider timeouts (30 s connect, 120 s overall).
pub fn provider_http_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(PROVIDER_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(PROVIDER_REQUEST_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// Return the per-process session affinity ID for cache-locality routing.
pub fn session_affinity_id() -> &'static str {
    &SESSION_AFFINITY_ID
}

/// Apply session affinity and custom headers to a request builder.
pub(crate) fn apply_custom_headers(
    mut req: reqwest::RequestBuilder,
    custom_headers: &std::collections::HashMap<String, String>,
) -> reqwest::RequestBuilder {
    req = req.header("x-session-affinity", session_affinity_id());
    for (k, v) in custom_headers {
        req = req.header(k.as_str(), v.as_str());
    }
    req
}

/// Shared warmup implementation for all providers.
///
/// Sends a minimal request to establish a connection and warm up any
/// server-side caches. Non-success responses are logged but not treated
/// as failures.
pub(crate) async fn warmup_provider(
    client: &Client,
    url: &str,
    headers: Vec<(&str, String)>,
    payload: Value,
    provider_name: &str,
) -> Result<Duration> {
    let start = std::time::Instant::now();
    let mut req = client
        .post(url)
        .header("x-session-affinity", session_affinity_id())
        .timeout(Duration::from_secs(15));
    for (key, value) in headers {
        req = req.header(key, value);
    }
    let result = req.json(&payload).send().await;
    match result {
        Ok(resp) if !resp.status().is_success() => {
            warn!(
                "{} warmup got HTTP {} (non-fatal)",
                provider_name,
                resp.status()
            );
        }
        Ok(_) => info!(
            "{} provider warmed up in {}ms",
            provider_name,
            start.elapsed().as_millis()
        ),
        Err(e) => warn!("{} warmup request failed (non-fatal): {}", provider_name, e),
    }
    Ok(start.elapsed())
}
