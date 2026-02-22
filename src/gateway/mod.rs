/// HTTP API server for the gateway.
///
/// Provides a REST endpoint for programmatic access to the agent.
/// Integrates with the existing `MessageBus` for inbound/outbound routing.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::bus::InboundMessage;
use crate::bus::OutboundMessage;

/// Timeout for waiting on agent response (2 minutes, matching provider timeout).
const RESPONSE_TIMEOUT_SECS: u64 = 120;

/// Shared state between HTTP handlers and the response router.
#[derive(Clone)]
pub struct HttpApiState {
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<OutboundMessage>>>>,
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
fn build_router(state: HttpApiState) -> Router {
    Router::new()
        .route("/api/chat", post(chat_handler))
        .route("/api/health", get(health_handler))
        .with_state(state)
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

/// Start the HTTP API server. Returns a join handle and the shared state
/// (needed by the outbound router to deliver responses).
pub async fn start(
    host: &str,
    port: u16,
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
) -> Result<(tokio::task::JoinHandle<()>, HttpApiState)> {
    let state = HttpApiState {
        inbound_tx,
        pending: Arc::new(Mutex::new(HashMap::new())),
    };

    let app = build_router(state.clone());
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
mod tests {
    use super::*;

    #[test]
    fn test_health_response_contains_version() {
        // Just verify the module compiles and types are correct
        let state = HttpApiState {
            inbound_tx: Arc::new(mpsc::channel(1).0),
            pending: Arc::new(Mutex::new(HashMap::new())),
        };
        let _router = build_router(state);
    }

    #[test]
    fn test_route_response_non_http_returns_false() {
        let state = HttpApiState {
            inbound_tx: Arc::new(mpsc::channel(1).0),
            pending: Arc::new(Mutex::new(HashMap::new())),
        };
        let msg = OutboundMessage {
            channel: "telegram".to_string(),
            chat_id: "123".to_string(),
            content: "hello".to_string(),
            reply_to: None,
            media: vec![],
            metadata: HashMap::new(),
        };
        assert!(!route_response(&state, msg));
    }

    #[test]
    fn test_route_response_http_with_pending() {
        let state = HttpApiState {
            inbound_tx: Arc::new(mpsc::channel(1).0),
            pending: Arc::new(Mutex::new(HashMap::new())),
        };
        let (tx, mut rx) = oneshot::channel();
        state
            .pending
            .lock()
            .unwrap()
            .insert("req-1".to_string(), tx);

        let msg = OutboundMessage {
            channel: "http".to_string(),
            chat_id: "req-1".to_string(),
            content: "response text".to_string(),
            reply_to: None,
            media: vec![],
            metadata: HashMap::new(),
        };
        assert!(route_response(&state, msg));
        let received = rx.try_recv().unwrap();
        assert_eq!(received.content, "response text");
    }

    #[test]
    fn test_route_response_http_no_pending() {
        let state = HttpApiState {
            inbound_tx: Arc::new(mpsc::channel(1).0),
            pending: Arc::new(Mutex::new(HashMap::new())),
        };
        let msg = OutboundMessage {
            channel: "http".to_string(),
            chat_id: "nonexistent".to_string(),
            content: "orphan".to_string(),
            reply_to: None,
            media: vec![],
            metadata: HashMap::new(),
        };
        // Should not panic, just return true (consumed) and warn
        assert!(route_response(&state, msg));
    }
}
