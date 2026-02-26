pub mod a2a;

/// HTTP API server for the gateway.
///
/// Provides REST endpoints for programmatic access to the agent and
/// generic webhook receivers for external service integrations.
/// Integrates with the existing `MessageBus` for inbound/outbound routing.
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
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

/// Max webhook payload size: 1 MB.
const WEBHOOK_MAX_BODY: usize = 1_048_576;

/// Max message size for chat API and A2A endpoints: 1 MB.
const MAX_MESSAGE_SIZE: usize = 1_048_576;

/// Timeout for waiting on agent response (2 minutes, matching provider timeout).
const RESPONSE_TIMEOUT_SECS: u64 = 120;

/// Shared state between HTTP handlers and the response router.
#[derive(Clone)]
pub struct HttpApiState {
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<OutboundMessage>>>>,
    webhooks: Arc<HashMap<String, WebhookConfig>>,
    outbound_tx: Option<Arc<mpsc::Sender<OutboundMessage>>>,
}

/// Request body for POST /api/chat.
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    /// The message content to send to the agent.
    pub message: String,
    /// Optional session ID for conversation continuity.
    /// If omitted, each request gets a unique session.
    pub session_id: Option<String>,
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

/// Build the HTTP API router.
fn build_router(state: HttpApiState, a2a_state: Option<a2a::A2aState>) -> Router {
    let mut router = Router::new()
        .route("/api/chat", post(chat_handler))
        .route("/api/health", get(health_handler))
        .route("/api/webhook/{name}", post(webhook_handler))
        .with_state(state);

    if let Some(a2a) = a2a_state {
        let a2a_router = Router::new()
            .route("/.well-known/agent.json", get(a2a::agent_card_handler))
            .route("/a2a/tasks", post(a2a::create_task_handler))
            .route("/a2a/tasks/{id}", get(a2a::get_task_handler))
            .with_state(a2a);
        router = router.merge(a2a_router);
    }

    router
}

/// POST /api/chat — send a message and receive the agent's response.
async fn chat_handler(
    State(state): State<HttpApiState>,
    Json(body): Json<ChatRequest>,
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
        let mut pending = state
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.insert(request_id.clone(), tx);
    }

    // Publish inbound message to the agent
    let msg = InboundMessage {
        channel: "http".to_string(),
        sender_id: "http-api".to_string(),
        chat_id: request_id.clone(),
        content: body.message,
        timestamp: chrono::Utc::now(),
        media: vec![],
        metadata: {
            let mut meta = HashMap::new();
            meta.insert(
                "session_id".to_string(),
                serde_json::Value::String(session_id.clone()),
            );
            meta
        },
    };

    if let Err(e) = state.inbound_tx.send(msg).await {
        // Clean up pending entry
        let mut pending = state
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            let mut pending = state
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
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
/// JSON keys are expanded first, then `{{body}}` last, so that attacker-controlled
/// body text cannot introduce secondary `{{key}}` expansions.
fn apply_template(template: &str, body_str: &str, json: Option<&serde_json::Value>) -> String {
    let mut result = template.to_string();
    if let Some(serde_json::Value::Object(map)) = json {
        for (key, value) in map {
            let placeholder = format!("{{{{{}}}}}", key);
            let replacement = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            result = result.replace(&placeholder, &replacement);
        }
    }
    result.replace("{{body}}", body_str)
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
        warn!("webhook {}: invalid signature", name);
        return StatusCode::FORBIDDEN.into_response();
    }

    debug!(
        "webhook {}: signature valid, payload_len={}",
        name,
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
            let mut pending = state
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            pending.insert(request_id.clone(), tx);
        }

        let inbound = InboundMessage {
            channel: "http".to_string(),
            sender_id: format!("webhook:{}", name),
            chat_id: request_id.clone(),
            content: message,
            timestamp: chrono::Utc::now(),
            media: vec![],
            metadata: {
                let mut meta = HashMap::new();
                meta.insert(
                    "webhook_name".to_string(),
                    serde_json::Value::String(name.clone()),
                );
                meta
            },
        };

        if let Err(e) = state.inbound_tx.send(inbound).await {
            let mut pending = state
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                let mut pending = state
                    .pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    for target in targets {
        let msg = OutboundMessage {
            channel: target.channel.clone(),
            chat_id: target.chat_id.clone(),
            content: content.to_string(),
            reply_to: None,
            media: vec![],
            metadata: {
                let mut meta = HashMap::new();
                meta.insert(
                    "webhook_source".to_string(),
                    serde_json::Value::String(webhook_name.to_string()),
                );
                meta
            },
        };
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
pub async fn start<S: BuildHasher>(
    host: &str,
    port: u16,
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
    outbound_tx: Option<Arc<mpsc::Sender<OutboundMessage>>>,
    webhooks: HashMap<String, WebhookConfig, S>,
    a2a_config: Option<crate::config::A2aConfig>,
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

    let state = HttpApiState {
        inbound_tx: inbound_tx.clone(),
        pending: pending.clone(),
        webhooks: Arc::new(webhook_map),
        outbound_tx,
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

    let app = build_router(state.clone(), a2a_state);
    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("HTTP API listening on {}", addr);

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
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

    let mut pending = state
        .pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
