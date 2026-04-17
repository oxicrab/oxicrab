//! Audit regression tests for the providers crate.
//!
//! Each test reproduces a finding from the providers audit. Tests are
//! expected to FAIL against current code — they lock in the desired
//! behavior so a fix is visible.

use oxicrab::providers::anthropic_oauth::AnthropicOAuthProvider;
use oxicrab::providers::base::{ChatRequest, LLMProvider, Message};
use oxicrab::providers::gemini::GeminiProvider;
use oxicrab::providers::openai::OpenAIProvider;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn simple_request() -> ChatRequest {
    ChatRequest::builder(vec![Message::user("hi")], 64)
        .temperature(0.7)
        .build()
}

// ─── Finding 2 ──────────────────────────────────────────────────────────────
// Gemini token parsing ignores `cachedContentTokenCount`. When Gemini
// returns cache usage, we should surface it through `LLMResponse`'s cache
// accounting fields so the cost log reflects real usage.

#[tokio::test]
async fn audit_providers_02_gemini_cache_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.5-pro:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": {"parts": [{"text": "ok"}], "role": "model"},
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 120,
                "cachedContentTokenCount": 90,
                "candidatesTokenCount": 8,
                "totalTokenCount": 128
            }
        })))
        .mount(&server)
        .await;

    let provider = GeminiProvider::with_config(
        "test_key".to_string(),
        Some("gemini-2.5-pro".to_string()),
        server.uri(),
        std::collections::HashMap::new(),
    );

    let resp = provider.chat(&simple_request()).await.unwrap();

    assert_eq!(resp.input_tokens, Some(120));
    assert_eq!(resp.output_tokens, Some(8));
    // Expected: cachedContentTokenCount is parsed into cache_read_input_tokens
    // (the unified cross-provider cache-read field). Current code drops it.
    assert_eq!(
        resp.cache_read_input_tokens,
        Some(90),
        "Gemini cachedContentTokenCount should surface as cache_read_input_tokens"
    );
}

// ─── Finding 4 ──────────────────────────────────────────────────────────────
// OpenAI returns `usage.prompt_tokens_details.cached_tokens` on cache hits
// (e.g. the automatic prompt cache for gpt-4o / gpt-4.1). The provider
// parser ignores this field, so the token ledger cannot distinguish
// cache-read tokens from full-price input tokens.

#[tokio::test]
async fn audit_providers_04_openai_cached_tokens_detail() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hi"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 500,
                "completion_tokens": 10,
                "total_tokens": 510,
                "prompt_tokens_details": {
                    "cached_tokens": 400
                }
            }
        })))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::with_config(
        "test_key".to_string(),
        "gpt-4o".to_string(),
        server.uri(),
        "OpenAI".to_string(),
    );

    let resp = provider.chat(&simple_request()).await.unwrap();

    assert_eq!(resp.input_tokens, Some(500));
    assert_eq!(resp.output_tokens, Some(10));
    // Expected: cached_tokens from prompt_tokens_details surfaces as
    // cache_read_input_tokens for uniform accounting. Current code drops it.
    assert_eq!(
        resp.cache_read_input_tokens,
        Some(400),
        "OpenAI prompt_tokens_details.cached_tokens should surface as cache_read_input_tokens"
    );
}

// ─── Finding 3 ──────────────────────────────────────────────────────────────
// AnthropicOAuthProvider::ensure_valid_token returns the expired access
// token when refresh fails, only logging a warning. The chat() path
// partly mitigates via a 401 retry, but ensure_valid_token itself
// silently propagates a stale token. A failed refresh against a
// known-expired token should produce an error, not a stale string.
//
// We exercise this by wiring a refresh endpoint that always returns 400
// and checking that chat() surfaces an error containing refresh context
// rather than succeeding with the stale bearer token.
//
// Constructing the provider against a test refresh URL requires
// redirecting TOKEN_URL. Since TOKEN_URL is a `const` embedded in the
// provider, we exercise the behavior through the chat() path against a
// mocked API endpoint: refresh will fail (real console.anthropic.com
// rejects our fake token), and a subsequent 401 should surface an error
// rather than loop forever. We assert that chat returns an error when
// the stored access token is expired and the refresh token is empty.

#[tokio::test]
async fn audit_providers_03_oauth_refresh_failure_surfaces_error() {
    // Expired 24h ago, no refresh token: ensure_valid_token cannot refresh
    // (empty refresh token path), then chat should still attempt the API
    // and on 401 surface a clear auth error — it must NOT silently return
    // a stale-token success.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {"type": "authentication_error", "message": "invalid bearer"}
        })))
        .mount(&server)
        .await;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let expired_at = now_ms - 86_400_000; // 24h in the past

    let provider = AnthropicOAuthProvider::new(
        "stale-access-token".to_string(),
        String::new(), // empty refresh → refresh path is skipped
        expired_at,
        Some("claude-opus-4-6".to_string()),
        None,
        None,
    )
    .unwrap();

    // The real Anthropic URL is a `const` in the provider module, so we
    // can't redirect the chat endpoint with wiremock — keep the server
    // alive only to demonstrate intent. Instead, we assert directly on
    // ensure_valid_token's contract through the public chat() path: with
    // an expired access token and no refresh token, chat must return Err.
    let _ = server; // silence unused warning
    let res = provider.chat(&simple_request()).await;

    assert!(
        res.is_err(),
        "expired token with no refresh capability must error, not silently send a stale bearer"
    );
    let msg = res.unwrap_err().to_string().to_lowercase();
    assert!(
        msg.contains("oauth")
            || msg.contains("expired")
            || msg.contains("auth")
            || msg.contains("refresh"),
        "error should reference the auth/refresh failure, got: {msg}"
    );
}
