pub mod anthropic;
pub mod anthropic_common;
pub mod anthropic_oauth;
pub mod base;
pub mod circuit_breaker;
pub mod errors;
pub mod fallback;
pub mod gemini;
pub mod openai;
pub mod prompt_guided;
pub mod strategy;
pub mod verify;

use reqwest::Client;
use std::sync::LazyLock;
use std::time::Duration;

/// Connect timeout for LLM provider HTTP clients (seconds).
pub(crate) const PROVIDER_CONNECT_TIMEOUT_SECS: u64 = 30;
/// Overall request timeout for LLM provider HTTP clients (seconds).
pub(crate) const PROVIDER_REQUEST_TIMEOUT_SECS: u64 = 120;

/// Per-process session affinity ID. Load balancers can use this to route
/// requests from the same process to the same backend for prompt cache locality.
static SESSION_AFFINITY_ID: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());

/// Build a `reqwest::Client` with standard provider timeouts (30 s connect, 120 s overall).
pub(crate) fn provider_http_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(PROVIDER_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(PROVIDER_REQUEST_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// Return the per-process session affinity ID for cache-locality routing.
pub(crate) fn session_affinity_id() -> &'static str {
    &SESSION_AFFINITY_ID
}
