pub mod a2a;

/// HTTP API server for the gateway.
///
/// Provides REST endpoints for programmatic access to the agent and
/// generic webhook receivers for external service integrations.
/// Integrates with the existing `MessageBus` for inbound/outbound routing.
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use governor::{DefaultKeyedRateLimiter, Quota};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::bus::InboundMessage;
use crate::bus::OutboundMessage;
use crate::config::schema::{WebhookConfig, WebhookTarget};

type HmacSha256 = Hmac<Sha256>;

/// Serialize a provider-level `ResponseFormat` to a JSON value for metadata transport.
fn response_format_to_json(rf: &crate::providers::base::ResponseFormat) -> serde_json::Value {
    match rf {
        crate::providers::base::ResponseFormat::JsonObject => {
            serde_json::Value::String("json".to_string())
        }
        crate::providers::base::ResponseFormat::JsonSchema { name, schema } => {
            serde_json::json!({ "name": name, "schema": schema })
        }
    }
}

/// Deserialize a provider-level `ResponseFormat` from a metadata JSON value.
pub(crate) fn response_format_from_json(
    v: &serde_json::Value,
) -> Option<crate::providers::base::ResponseFormat> {
    match v {
        serde_json::Value::String(s) if s == "json" => {
            Some(crate::providers::base::ResponseFormat::JsonObject)
        }
        serde_json::Value::Object(map) => {
            let name = map.get("name")?.as_str()?.to_string();
            let schema = map.get("schema")?.clone();
            Some(crate::providers::base::ResponseFormat::JsonSchema { name, schema })
        }
        _ => None,
    }
}

/// Max webhook payload size: 1 MB.
const WEBHOOK_MAX_BODY: usize = 1_048_576;

/// Max message size for chat API and A2A endpoints: 1 MB.
const MAX_MESSAGE_SIZE: usize = 1_048_576;

/// Max size for JSON schema in response format: 100 KB.
/// This prevents uncontrolled allocation from user-provided schemas.
const MAX_SCHEMA_SIZE: usize = 100 * 1024;

/// Max length for response format schema names and simple format strings.
const MAX_FORMAT_NAME_LEN: usize = 256;

/// Timeout for waiting on agent response (2 minutes, matching provider timeout).
const RESPONSE_TIMEOUT_SECS: u64 = 120;

/// Maximum age of webhook timestamps for replay protection (5 minutes).
const REPLAY_WINDOW_SECS: i64 = 300;

/// Shared state between HTTP handlers and the response router.
#[derive(Clone)]
pub struct HttpApiState {
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<OutboundMessage>>>>,
    webhooks: Arc<HashMap<String, WebhookConfig>>,
    outbound_tx: Option<Arc<mpsc::Sender<OutboundMessage>>>,
    /// Pre-configured leak detector with known secrets registered.
    /// Used by `deliver_to_targets()` which bypasses `MessageBus` and must
    /// run its own leak detection.
    leak_detector: Arc<crate::safety::leak_detector::LeakDetector>,
    /// Set to `true` once the agent loop is fully initialized and running.
    /// The health endpoint uses this to distinguish liveness from readiness.
    ready: Arc<AtomicBool>,
}

/// Drop guard that removes a pending response entry when the handler is dropped
/// (e.g., on client disconnect). If the response already arrived via `route_response()`,
/// the entry will already be consumed and the remove is a harmless no-op.
pub(crate) struct PendingCleanup {
    pub(crate) pending: Arc<Mutex<HashMap<String, oneshot::Sender<OutboundMessage>>>>,
    pub(crate) id: String,
}

impl Drop for PendingCleanup {
    fn drop(&mut self) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.remove(&self.id);
    }
}

/// Request body for POST /api/chat.
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    /// The message content to send to the agent.
    pub message: String,
    /// Optional session ID for conversation continuity.
    /// If omitted, each request gets a unique session.
    pub session_id: Option<String>,
    /// Request structured JSON output from the LLM.
    ///
    /// Accepts:
    /// - `"json"` — request unstructured JSON output
    /// - `{"name": "...", "schema": {...}}` — request output matching a JSON schema
    ///
    /// When omitted, the LLM responds in its default format (prose).
    #[serde(default, rename = "responseFormat")]
    pub response_format: Option<GatewayResponseFormat>,
}

/// Gateway-level response format specification, parsed from the HTTP request body.
///
/// Converted to the provider-level [`ResponseFormat`](crate::providers::base::ResponseFormat)
/// before being passed to the agent loop.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum GatewayResponseFormat {
    /// Simple string: `"json"` for unstructured JSON output.
    Simple(String),
    /// JSON schema: `{"name": "...", "schema": {...}}` for structured output.
    Schema {
        name: String,
        schema: serde_json::Value,
    },
}

impl GatewayResponseFormat {
    /// Convert to the provider-level response format enum.
    fn into_response_format(self) -> crate::providers::base::ResponseFormat {
        match self {
            GatewayResponseFormat::Simple(s) if s == "json" => {
                crate::providers::base::ResponseFormat::JsonObject
            }
            GatewayResponseFormat::Simple(_) => {
                // Unrecognized string — fall back to JsonObject
                crate::providers::base::ResponseFormat::JsonObject
            }
            GatewayResponseFormat::Schema { name, schema } => {
                crate::providers::base::ResponseFormat::JsonSchema { name, schema }
            }
        }
    }
}

/// Response body for POST /api/chat.
#[derive(Debug, Serialize)]
pub struct ChatResponse {
    /// The agent's response text.
    pub content: String,
    /// The session ID (for follow-up requests).
    pub session_id: String,
}

/// Error response body.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// API key authentication middleware.
///
/// Checks `Authorization: Bearer <key>` or `X-API-Key: <key>` headers.
/// Returns 401 if the key is missing or incorrect.
async fn api_key_auth(
    State(expected): State<Arc<String>>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> axum::response::Response {
    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            // RFC 7235: token type is case-insensitive
            if v.len() > 7 && v[..7].eq_ignore_ascii_case("bearer ") {
                Some(v[7..].trim())
            } else {
                None
            }
        })
        .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()));

    match provided {
        Some(key) if key.as_bytes().ct_eq(expected.as_bytes()).into() => next.run(request).await,
        _ => {
            warn!("rejected unauthenticated request to {}", request.uri());
            (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "unauthorized — set Authorization: Bearer <apiKey> or X-API-Key header"
                        .to_string(),
                }),
            )
                .into_response()
        }
    }
}

/// Shared state for the rate limiting middleware.
#[derive(Clone)]
struct RateLimitState {
    limiter: Arc<DefaultKeyedRateLimiter<String>>,
    trust_proxy: bool,
    /// Retry-After value derived from 1/rps (seconds), minimum 1.
    retry_after_secs: u64,
}

/// Per-IP rate limiting middleware using governor.
///
/// Uses the actual socket peer address by default. Only falls back to
/// X-Forwarded-For when `trust_proxy` is enabled (for reverse-proxy setups).
/// Exempts `/api/health` from rate limiting.
async fn rate_limit_middleware(
    State(state): State<RateLimitState>,
    request: Request,
    next: Next,
) -> axum::response::Response {
    // Skip rate limiting for health endpoint
    if request.uri().path() == "/api/health" {
        return next.run(request).await;
    }

    let connect_info = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .copied();
    let socket_ip = connect_info.map(|ci| ci.0.ip().to_string());

    let ip = if state.trust_proxy {
        // Trust X-Forwarded-For when behind a reverse proxy
        request
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|s| s.trim().to_string())
            .or(socket_ip)
            .unwrap_or_else(|| "unknown".to_string())
    } else {
        // Default: use actual socket address (not spoofable)
        socket_ip.unwrap_or_else(|| "unknown".to_string())
    };

    if state.limiter.check_key(&ip).is_ok() {
        return next.run(request).await;
    }

    let retry_after = state.retry_after_secs.to_string();
    warn!("rate limit exceeded for {}", ip);
    (
        StatusCode::TOO_MANY_REQUESTS,
        [("retry-after", retry_after.as_str())],
        Json(ErrorResponse {
            error: "rate limit exceeded".to_string(),
        }),
    )
        .into_response()
}

/// Build the HTTP API router.
#[allow(clippy::needless_pass_by_value)] // api_key is Arc-cloned into middleware layers
fn build_router(
    state: HttpApiState,
    a2a_state: Option<a2a::A2aState>,
    api_key: Option<Arc<String>>,
    rate_limiter: Option<RateLimitState>,
) -> Router {
    // Routes that require auth when an API key is configured
    let mut authed_routes = Router::new()
        .route("/api/chat", post(chat_handler))
        .with_state(state.clone());

    if let Some(ref key) = api_key {
        authed_routes =
            authed_routes.layer(middleware::from_fn_with_state(key.clone(), api_key_auth));
    }

    // Public routes (health, webhooks with their own HMAC auth)
    let public_routes = Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/webhook/{name}", post(webhook_handler))
        .with_state(state);

    let mut router = authed_routes
        .merge(public_routes)
        .layer(DefaultBodyLimit::max(WEBHOOK_MAX_BODY));

    if let Some(a2a) = a2a_state {
        // A2A: agent card is public (discovery), task endpoints require auth
        let public_a2a = Router::new()
            .route("/.well-known/agent.json", get(a2a::agent_card_handler))
            .with_state(a2a.clone());

        let mut authed_a2a = Router::new()
            .route("/a2a/tasks", post(a2a::create_task_handler))
            .route("/a2a/tasks/{id}", get(a2a::get_task_handler))
            .with_state(a2a);

        if let Some(ref key) = api_key {
            authed_a2a =
                authed_a2a.layer(middleware::from_fn_with_state(key.clone(), api_key_auth));
        }

        let a2a_router = authed_a2a
            .merge(public_a2a)
            .layer(DefaultBodyLimit::max(MAX_MESSAGE_SIZE + 1024));

        router = router.merge(a2a_router);
    }

    if let Some(rl_state) = rate_limiter {
        router = router.layer(middleware::from_fn_with_state(
            rl_state,
            rate_limit_middleware,
        ));
    }

    router
}

/// POST /api/chat — send a message and receive the agent's response.
async fn chat_handler(
    State(state): State<HttpApiState>,
    Json(mut body): Json<ChatRequest>,
) -> impl IntoResponse {
    let session_id = body
        .session_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let request_id = format!("http-{}", Uuid::new_v4());

    if body.message.len() > MAX_MESSAGE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "message too large"})),
        );
    }

    debug!(
        "HTTP API chat request: session={}, content_len={}",
        session_id,
        body.message.len()
    );

    // Create oneshot channel for the response
    let (tx, rx) = oneshot::channel();
    {
        let mut pending = state.pending.lock().unwrap_or_else(|poison| {
            warn!("gateway pending map mutex was poisoned, recovering");
            poison.into_inner()
        });
        pending.insert(request_id.clone(), tx);
    }

    // Drop guard: remove pending entry if the handler is dropped (client disconnect).
    // When the response arrives normally, the entry is consumed by route_response()
    // so this guard's remove is a harmless no-op.
    let _cleanup = PendingCleanup {
        pending: state.pending.clone(),
        id: request_id.clone(),
    };

    // Convert gateway response format to provider-level enum and serialize
    // into metadata so the agent loop can extract it.
    // Validate schema size to prevent uncontrolled allocation.
    let response_format_value = if let Some(ref rf) = body.response_format {
        match rf {
            GatewayResponseFormat::Schema { name, schema } => {
                // Check schema size by serializing to estimate memory usage
                let schema_size = serde_json::to_string(schema).map_or(0, |s| s.len());
                if schema_size > MAX_SCHEMA_SIZE {
                    return (
                        StatusCode::PAYLOAD_TOO_LARGE,
                        Json(serde_json::json!({
                            "error": format!("response format schema too large (max {} bytes)", MAX_SCHEMA_SIZE)
                        })),
                    );
                }
                // Also check name length
                if name.len() > MAX_FORMAT_NAME_LEN {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": format!("response format schema name too long (max {} characters)", MAX_FORMAT_NAME_LEN)
                        })),
                    );
                }
            }
            GatewayResponseFormat::Simple(s) => {
                // Check simple string length
                if s.len() > MAX_FORMAT_NAME_LEN {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": format!("response format value too long (max {} characters)", MAX_FORMAT_NAME_LEN)
                        })),
                    );
                }
            }
        }
        // Validation passed, now convert
        body.response_format
            .take()
            .map(GatewayResponseFormat::into_response_format)
    } else {
        None
    };

    // Publish inbound message to the agent
    let mut builder = InboundMessage::builder("http", "http-api", request_id.clone(), body.message)
        .meta(
            crate::bus::meta::SESSION_ID,
            serde_json::Value::String(session_id.clone()),
        );
    if let Some(ref rf) = response_format_value {
        builder = builder.meta(
            crate::bus::meta::RESPONSE_FORMAT,
            response_format_to_json(rf),
        );
    }
    let msg = builder.build();

    if let Err(e) = state.inbound_tx.send(msg).await {
        // Clean up pending entry
        let mut pending = state.pending.lock().unwrap_or_else(|poison| {
            warn!("gateway pending map mutex was poisoned, recovering");
            poison.into_inner()
        });
        pending.remove(&request_id);
        error!("failed to publish HTTP API message: {}", e);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "agent unavailable"})),
        );
    }

    // Wait for the agent's response with timeout
    match tokio::time::timeout(Duration::from_secs(RESPONSE_TIMEOUT_SECS), rx).await {
        Ok(Ok(response)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "content": response.content,
                "session_id": session_id
            })),
        ),
        Ok(Err(_)) => {
            warn!(
                "HTTP API response channel closed for request {}",
                request_id
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "response channel closed"})),
            )
        }
        Err(_) => {
            // Timeout — clean up pending entry
            let mut pending = state.pending.lock().unwrap_or_else(|poison| {
                warn!("gateway pending map mutex was poisoned, recovering");
                poison.into_inner()
            });
            pending.remove(&request_id);
            warn!("HTTP API request timed out: {}", request_id);
            (
                StatusCode::GATEWAY_TIMEOUT,
                Json(serde_json::json!({"error": "request timed out"})),
            )
        }
    }
}

/// GET /api/health — health check endpoint.
///
/// Returns `"ready"` once the agent loop is running, `"starting"` during
/// initialization. Kubernetes-style probes: use `/api/health` for liveness
/// (always 200) and check `status == "ready"` for readiness.
async fn health_handler(State(state): State<HttpApiState>) -> impl IntoResponse {
    let is_ready = state.ready.load(Ordering::SeqCst);
    Json(serde_json::json!({
        "status": if is_ready { "ready" } else { "starting" },
        "version": crate::VERSION
    }))
}

/// Validate HMAC-SHA256 signature against a payload.
pub(crate) fn validate_webhook_signature(secret: &str, signature: &str, body: &[u8]) -> bool {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let result = mac.finalize();
    let expected = hex::encode(result.into_bytes());

    // Support both raw hex and "sha256=..." prefix (GitHub-style)
    let sig = signature.strip_prefix("sha256=").unwrap_or(signature);
    expected.as_bytes().ct_eq(sig.as_bytes()).into()
}

/// Apply a template string, substituting `{{key}}` with JSON payload values.
/// `{{body}}` is replaced with the raw body string.
///
/// Uses a single-pass approach: splits the template on `{{...}}` boundaries
/// and looks up each placeholder, so replacement values are never re-scanned
/// for further `{{key}}` patterns.
fn apply_template(template: &str, body_str: &str, json: Option<&serde_json::Value>) -> String {
    let map = match json {
        Some(serde_json::Value::Object(m)) => Some(m),
        _ => None,
    };

    let mut result = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        result.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        if let Some(end) = after_open.find("}}") {
            let key = &after_open[..end];
            if key == "body" {
                result.push_str(body_str);
            } else if let Some(m) = map {
                if let Some(value) = m.get(key) {
                    match value {
                        serde_json::Value::String(s) => result.push_str(s),
                        other => result.push_str(&other.to_string()),
                    }
                } else {
                    result.push_str("{{");
                    result.push_str(key);
                    result.push_str("}}");
                }
            } else {
                result.push_str("{{");
                result.push_str(key);
                result.push_str("}}");
            }
            rest = &after_open[end + 2..];
        } else {
            result.push_str("{{");
            rest = after_open;
        }
        // Cap output size to prevent template expansion amplification
        if result.len() > WEBHOOK_MAX_BODY {
            warn!(
                "webhook template output exceeded {}B limit, truncating",
                WEBHOOK_MAX_BODY
            );
            let mut end = WEBHOOK_MAX_BODY;
            while !result.is_char_boundary(end) {
                end -= 1;
            }
            result.truncate(end);
            return result;
        }
    }
    result.push_str(rest);
    result
}

/// POST /api/webhook/{name} — receive a webhook from an external service.
async fn webhook_handler(
    State(state): State<HttpApiState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Look up webhook config
    let Some(config) = state.webhooks.get(&name) else {
        debug!("webhook: unknown webhook name={}", name);
        return StatusCode::NOT_FOUND.into_response();
    };

    if !config.enabled {
        debug!("webhook: disabled webhook name={}", name);
        return StatusCode::NOT_FOUND.into_response();
    }

    // Reject webhooks with empty secrets (deny-by-default). An empty HMAC
    // secret would accept any signature, so we refuse to process the request
    // until the operator configures a proper secret.
    if config.secret.is_empty() {
        warn!("webhook {name}: no secret configured, rejecting all requests");
        return StatusCode::FORBIDDEN.into_response();
    }

    // Enforce max body size
    if body.len() > WEBHOOK_MAX_BODY {
        warn!("webhook {}: payload too large ({} bytes)", name, body.len());
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }

    // Extract signature from headers (check common header names)
    let signature = headers
        .get("X-Signature-256")
        .or_else(|| headers.get("X-Hub-Signature-256"))
        .or_else(|| headers.get("X-Webhook-Signature"))
        .and_then(|v| v.to_str().ok());

    let Some(signature) = signature else {
        warn!("webhook {}: missing signature header", name);
        return StatusCode::FORBIDDEN.into_response();
    };

    // Validate HMAC-SHA256 signature
    if !validate_webhook_signature(&config.secret, signature, &body) {
        warn!("webhook {name}: invalid signature");
        return StatusCode::FORBIDDEN.into_response();
    }

    // Replay protection: reject payloads with timestamps older than 5 minutes.
    // Checks X-Webhook-Timestamp (Unix seconds) if present.
    // NOTE: The timestamp is NOT included in the HMAC signature, so a captured
    // body+signature can be replayed with a fresh timestamp. For stronger replay
    // protection, webhook senders should include the timestamp in the HMAC input.
    // This check is defense-in-depth for senders that include timestamps.
    if let Some(ts_header) = headers
        .get("X-Webhook-Timestamp")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
    {
        let now = chrono::Utc::now().timestamp();
        if (now - ts_header).abs() > REPLAY_WINDOW_SECS {
            warn!("webhook {name}: timestamp too old ({ts_header}), rejecting (replay?)");
            return StatusCode::FORBIDDEN.into_response();
        }
    }

    debug!(
        "webhook {name}: signature valid, payload_len={}",
        body.len()
    );

    // Parse body as string and optionally as JSON
    let body_str = String::from_utf8_lossy(&body);
    let json_value: Option<serde_json::Value> = serde_json::from_slice(&body).ok();

    // Apply template
    let message = apply_template(&config.template, &body_str, json_value.as_ref());

    if config.agent_turn {
        // Route through agent loop — publish as inbound message and wait for response
        let request_id = format!("webhook-{}-{}", name, Uuid::new_v4());

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = state.pending.lock().unwrap_or_else(|poison| {
                warn!("gateway pending map mutex was poisoned, recovering");
                poison.into_inner()
            });
            pending.insert(request_id.clone(), tx);
        }

        // Drop guard: remove pending entry if the handler is dropped or panics.
        let _cleanup = PendingCleanup {
            pending: state.pending.clone(),
            id: request_id.clone(),
        };

        let inbound = InboundMessage::builder(
            "http",
            format!("webhook:{name}"),
            request_id.clone(),
            message,
        )
        .meta(
            crate::bus::meta::WEBHOOK_NAME,
            serde_json::Value::String(name.clone()),
        )
        .build();

        if let Err(e) = state.inbound_tx.send(inbound).await {
            let mut pending = state.pending.lock().unwrap_or_else(|poison| {
                warn!("gateway pending map mutex was poisoned, recovering");
                poison.into_inner()
            });
            pending.remove(&request_id);
            error!("webhook {}: failed to publish inbound message: {}", name, e);
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }

        // Wait for agent response, then forward to targets
        match tokio::time::timeout(Duration::from_secs(RESPONSE_TIMEOUT_SECS), rx).await {
            Ok(Ok(response)) => {
                deliver_to_targets(&state, &config.targets, &response.content, &name).await;
                Json(serde_json::json!({
                    "status": "ok",
                    "delivered": true
                }))
                .into_response()
            }
            Ok(Err(_)) => {
                warn!("webhook {}: response channel closed", name);
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
            Err(_) => {
                let mut pending = state.pending.lock().unwrap_or_else(|poison| {
                    warn!("gateway pending map mutex was poisoned, recovering");
                    poison.into_inner()
                });
                pending.remove(&request_id);
                warn!("webhook {}: agent response timed out", name);
                StatusCode::GATEWAY_TIMEOUT.into_response()
            }
        }
    } else {
        // Direct delivery — send templated message to targets without agent
        deliver_to_targets(&state, &config.targets, &message, &name).await;
        Json(serde_json::json!({
            "status": "ok",
            "delivered": true
        }))
        .into_response()
    }
}

/// Deliver a message to configured webhook targets via the outbound channel.
async fn deliver_to_targets(
    state: &HttpApiState,
    targets: &[WebhookTarget],
    content: &str,
    webhook_name: &str,
) {
    let Some(ref outbound_tx) = state.outbound_tx else {
        warn!(
            "webhook {}: no outbound sender configured, cannot deliver to targets",
            webhook_name
        );
        return;
    };

    // Scan for leaked secrets before delivery (this path bypasses MessageBus,
    // so we must run leak detection ourselves). Uses the pre-configured
    // detector that has known secrets registered, matching what MessageBus does.
    let safe_content = state.leak_detector.redact(content);
    if safe_content != content {
        warn!("webhook {webhook_name}: redacted leaked secrets from target delivery");
    }

    for target in targets {
        let msg = OutboundMessage::builder(
            target.channel.clone(),
            target.chat_id.clone(),
            safe_content.clone(),
        )
        .meta(
            crate::bus::meta::WEBHOOK_SOURCE,
            serde_json::Value::String(webhook_name.to_string()),
        )
        .build();
        if let Err(e) = outbound_tx.send(msg).await {
            error!(
                "webhook {}: failed to deliver to {}:{}: {}",
                webhook_name, target.channel, target.chat_id, e
            );
        } else {
            debug!(
                "webhook {}: delivered to {}:{}",
                webhook_name, target.channel, target.chat_id
            );
        }
    }
}

/// Start the HTTP API server. Returns a join handle and the shared state
/// (needed by the outbound router to deliver responses).
///
/// `known_secrets` are registered with the leak detector used by
/// `deliver_to_targets()` — this path bypasses `MessageBus` so it needs
/// its own detector with the same secrets.
#[allow(clippy::too_many_arguments)]
pub async fn start<S: BuildHasher>(
    host: &str,
    port: u16,
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
    outbound_tx: Option<Arc<mpsc::Sender<OutboundMessage>>>,
    webhooks: HashMap<String, WebhookConfig, S>,
    a2a_config: Option<crate::config::A2aConfig>,
    api_key: Option<String>,
    rate_limit: &crate::config::schema::RateLimitConfig,
    known_secrets: &[(&str, &str)],
    ready: Arc<AtomicBool>,
) -> Result<(tokio::task::JoinHandle<()>, HttpApiState)> {
    let webhook_map: HashMap<String, WebhookConfig> = webhooks.into_iter().collect();
    let active: Vec<_> = webhook_map
        .iter()
        .filter(|(_, v)| v.enabled)
        .map(|(k, _)| k.clone())
        .collect();
    if !active.is_empty() {
        info!(
            "registered {} webhook endpoint(s): {}",
            active.len(),
            active.join(", ")
        );
    }

    let pending = Arc::new(Mutex::new(HashMap::new()));

    let mut detector = crate::safety::leak_detector::LeakDetector::new();
    if !known_secrets.is_empty() {
        detector.add_known_secrets(known_secrets);
        debug!(
            "gateway leak detector registered {} known secret(s)",
            known_secrets.len()
        );
    }

    let state = HttpApiState {
        inbound_tx: inbound_tx.clone(),
        pending: pending.clone(),
        webhooks: Arc::new(webhook_map),
        outbound_tx,
        leak_detector: Arc::new(detector),
        ready,
    };

    // Set up A2A state if enabled
    let a2a_state = match a2a_config {
        Some(cfg) if cfg.enabled => {
            info!("A2A protocol enabled");
            Some(a2a::A2aState {
                config: cfg,
                store: Arc::new(a2a::A2aTaskStore::new()),
                inbound_tx,
                pending,
                host: host.to_string(),
                port,
            })
        }
        _ => None,
    };

    let key = api_key.filter(|k| !k.is_empty()).map(Arc::new);
    if key.is_some() {
        info!("HTTP API authentication enabled (Bearer / X-API-Key)");
    }
    let rate_limiter = if rate_limit.enabled {
        let rps = NonZeroU32::new(rate_limit.requests_per_second).unwrap_or_else(|| {
            warn!("gateway rate limit requests_per_second is 0, using default 10");
            NonZeroU32::new(10).unwrap()
        });
        let burst = NonZeroU32::new(rate_limit.burst).unwrap_or_else(|| {
            warn!("gateway rate limit burst is 0, using default 20");
            NonZeroU32::new(20).unwrap()
        });
        let quota = Quota::per_second(rps).allow_burst(burst);
        let retry_after_secs = (1.0 / f64::from(rps.get())).ceil() as u64;
        let retry_after_secs = retry_after_secs.max(1);
        info!(
            "rate limiting enabled: {} req/s, burst {}{}",
            rps,
            burst,
            if rate_limit.trust_proxy {
                " (trusting X-Forwarded-For)"
            } else {
                ""
            }
        );
        Some(RateLimitState {
            limiter: Arc::new(governor::RateLimiter::keyed(quota)),
            trust_proxy: rate_limit.trust_proxy,
            retry_after_secs,
        })
    } else {
        None
    };
    let app = build_router(state.clone(), a2a_state, key, rate_limiter);
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("HTTP API listening on {}", addr);

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        {
            error!("HTTP API server error: {}", e);
        }
    });

    Ok((handle, state))
}

/// Route an outbound message to a pending HTTP API request.
/// Returns true if the message was consumed (i.e., it was an HTTP response).
pub fn route_response(state: &HttpApiState, msg: OutboundMessage) -> bool {
    if msg.channel != "http" {
        return false;
    }

    let mut pending = state.pending.lock().unwrap_or_else(|poison| {
        warn!("gateway pending map mutex was poisoned, recovering");
        poison.into_inner()
    });
    if let Some(tx) = pending.remove(&msg.chat_id) {
        if tx.send(msg).is_err() {
            warn!("HTTP API client disconnected before receiving response");
        }
        true
    } else {
        warn!("no pending HTTP API request for chat_id={}", msg.chat_id);
        true // Still consumed — don't route to channel manager
    }
}

#[cfg(test)]
mod tests;
