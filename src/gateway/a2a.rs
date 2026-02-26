use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::bus::{InboundMessage, OutboundMessage};
use crate::config::A2aConfig;

/// Timeout for A2A task processing (same as chat handler).
const A2A_TIMEOUT_SECS: u64 = 120;

/// Maximum number of A2A tasks retained in memory.
const MAX_A2A_TASKS: usize = 1000;

// --- AgentCard types ---

#[derive(Debug, Serialize)]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub url: String,
    pub capabilities: AgentCapabilities,
}

#[derive(Debug, Serialize)]
pub struct AgentCapabilities {
    pub content_types: Vec<String>,
}

// --- Task types ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Submitted,
    Working,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct A2aTask {
    pub id: String,
    pub status: TaskStatus,
    pub message: String,
    pub result: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// In-memory task store for A2A protocol.
#[derive(Default)]
pub struct A2aTaskStore {
    tasks: Mutex<HashMap<String, A2aTask>>,
}

impl A2aTaskStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn insert(&self, task: A2aTask) {
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Evict oldest task if at capacity
        if tasks.len() >= MAX_A2A_TASKS
            && let Some(oldest_id) = tasks
                .values()
                .min_by_key(|t| &t.created_at)
                .map(|t| t.id.clone())
        {
            tasks.remove(&oldest_id);
        }
        tasks.insert(task.id.clone(), task);
    }

    fn get(&self, id: &str) -> Option<A2aTask> {
        let tasks = self
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tasks.get(id).cloned()
    }

    fn update_status(&self, id: &str, status: TaskStatus, result: Option<String>) {
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(task) = tasks.get_mut(id) {
            task.status = status;
            task.result = result;
            task.updated_at = Utc::now().to_rfc3339();
        }
    }
}

/// Shared state for A2A handlers.
#[derive(Clone)]
pub struct A2aState {
    pub config: A2aConfig,
    pub store: Arc<A2aTaskStore>,
    pub inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
    pub pending: Arc<Mutex<HashMap<String, oneshot::Sender<OutboundMessage>>>>,
    pub host: String,
    pub port: u16,
}

/// Request body for POST /a2a/tasks.
#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    pub message: String,
}

/// Response for POST /a2a/tasks.
#[derive(Debug, Serialize)]
pub struct CreateTaskResponse {
    pub task_id: String,
    pub status: TaskStatus,
}

// --- Handlers ---

/// GET `/.well-known/agent.json` — returns the `AgentCard`.
pub async fn agent_card_handler(State(state): State<A2aState>) -> impl IntoResponse {
    let name = if state.config.agent_name.is_empty() {
        "oxicrab".to_string()
    } else {
        state.config.agent_name.clone()
    };
    let description = if state.config.agent_description.is_empty() {
        "AI assistant powered by oxicrab".to_string()
    } else {
        state.config.agent_description.clone()
    };
    let url = format!("http://{}:{}", state.host, state.port);

    let card = AgentCard {
        name,
        description,
        url,
        capabilities: AgentCapabilities {
            content_types: vec!["text/plain".to_string()],
        },
    };

    (StatusCode::OK, Json(card))
}

/// POST /a2a/tasks — submit a task for processing.
pub async fn create_task_handler(
    State(state): State<A2aState>,
    Json(body): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    if body.message.len() > super::MAX_MESSAGE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "message too large"})),
        );
    }

    let task_id = format!("a2a-{}", Uuid::new_v4());
    let now = Utc::now().to_rfc3339();

    let task = A2aTask {
        id: task_id.clone(),
        status: TaskStatus::Submitted,
        message: body.message.clone(),
        result: None,
        created_at: now.clone(),
        updated_at: now,
    };
    state.store.insert(task);

    debug!("A2A task created: {}", task_id);

    // Create oneshot channel for the response
    let (tx, rx) = oneshot::channel();
    {
        let mut pending = state
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.insert(task_id.clone(), tx);
    }

    // Update status to working
    state
        .store
        .update_status(&task_id, TaskStatus::Working, None);

    // Publish inbound message to the agent
    let msg = InboundMessage {
        channel: "http".to_string(),
        sender_id: "a2a".to_string(),
        chat_id: task_id.clone(),
        content: body.message,
        timestamp: Utc::now(),
        media: vec![],
        metadata: HashMap::new(),
    };

    if let Err(e) = state.inbound_tx.send(msg).await {
        let mut pending = state
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.remove(&task_id);
        state.store.update_status(
            &task_id,
            TaskStatus::Failed,
            Some(format!("agent unavailable: {e}")),
        );
        error!("A2A: failed to publish message: {}", e);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "agent unavailable"})),
        );
    }

    // Spawn async processing — response will update the task store
    let store = state.store.clone();
    let pending = state.pending.clone();
    let tid = task_id.clone();
    tokio::spawn(async move {
        match tokio::time::timeout(Duration::from_secs(A2A_TIMEOUT_SECS), rx).await {
            Ok(Ok(response)) => {
                store.update_status(&tid, TaskStatus::Completed, Some(response.content));
            }
            Ok(Err(_)) => {
                warn!("A2A response channel closed for task {}", tid);
                store.update_status(
                    &tid,
                    TaskStatus::Failed,
                    Some("response channel closed".to_string()),
                );
            }
            Err(_) => {
                // Clean up dead pending entry
                pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&tid);
                warn!("A2A task {} timed out after {}s", tid, A2A_TIMEOUT_SECS);
                store.update_status(&tid, TaskStatus::Failed, Some("timeout".to_string()));
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "task_id": task_id,
            "status": "submitted"
        })),
    )
}

/// GET /a2a/tasks/:id — get task status.
pub async fn get_task_handler(
    State(state): State<A2aState>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    match state.store.get(&task_id) {
        Some(task) => match serde_json::to_value(task) {
            Ok(val) => (StatusCode::OK, Json(val)),
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "serialization failed"})),
            ),
        },
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "task not found"})),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::{get, post};
    use tower::ServiceExt;

    fn make_a2a_state() -> (A2aState, mpsc::Receiver<InboundMessage>) {
        let (tx, rx) = mpsc::channel(16);
        let state = A2aState {
            config: A2aConfig {
                enabled: true,
                agent_name: "test-agent".to_string(),
                agent_description: "A test agent".to_string(),
            },
            store: Arc::new(A2aTaskStore::new()),
            inbound_tx: Arc::new(tx),
            pending: Arc::new(Mutex::new(HashMap::new())),
            host: "127.0.0.1".to_string(),
            port: 3000,
        };
        (state, rx)
    }

    fn a2a_router(state: A2aState) -> Router {
        Router::new()
            .route("/.well-known/agent.json", get(agent_card_handler))
            .route("/a2a/tasks", post(create_task_handler))
            .route("/a2a/tasks/{id}", get(get_task_handler))
            .with_state(state)
    }

    #[test]
    fn test_task_store_lifecycle() {
        let store = A2aTaskStore::new();
        let task = A2aTask {
            id: "test-1".to_string(),
            status: TaskStatus::Submitted,
            message: "hello".to_string(),
            result: None,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
        };
        store.insert(task);

        let retrieved = store.get("test-1").unwrap();
        assert_eq!(retrieved.status, TaskStatus::Submitted);
        assert!(retrieved.result.is_none());

        store.update_status("test-1", TaskStatus::Completed, Some("done".to_string()));
        let updated = store.get("test-1").unwrap();
        assert_eq!(updated.status, TaskStatus::Completed);
        assert_eq!(updated.result.as_deref(), Some("done"));
    }

    #[test]
    fn test_task_store_missing() {
        let store = A2aTaskStore::new();
        assert!(store.get("nonexistent").is_none());
    }

    #[test]
    fn test_task_status_serialize() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::Submitted).unwrap(),
            "\"submitted\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Working).unwrap(),
            "\"working\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Failed).unwrap(),
            "\"failed\""
        );
    }

    #[tokio::test]
    async fn test_agent_card_returns_config_values() {
        let (state, _rx) = make_a2a_state();
        let app = a2a_router(state);

        let req = Request::builder()
            .method("GET")
            .uri("/.well-known/agent.json")
            .body(Body::empty())
            .unwrap();

        let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "test-agent");
        assert_eq!(json["description"], "A test agent");
        assert_eq!(json["url"], "http://127.0.0.1:3000");
        assert!(
            json["capabilities"]["content_types"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "text/plain")
        );
    }

    #[tokio::test]
    async fn test_agent_card_defaults_when_empty() {
        let (mut state, _rx) = make_a2a_state();
        state.config.agent_name = String::new();
        state.config.agent_description = String::new();
        let app = a2a_router(state);

        let req = Request::builder()
            .method("GET")
            .uri("/.well-known/agent.json")
            .body(Body::empty())
            .unwrap();

        let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "oxicrab");
        assert_eq!(json["description"], "AI assistant powered by oxicrab");
    }

    #[tokio::test]
    async fn test_create_task_publishes_inbound_message() {
        let (state, mut rx) = make_a2a_state();
        let app = a2a_router(state.clone());

        let req = Request::builder()
            .method("POST")
            .uri("/a2a/tasks")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"message": "hello from A2A"}"#))
            .unwrap();

        let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let task_id = json["task_id"].as_str().unwrap();
        assert!(task_id.starts_with("a2a-"));
        assert_eq!(json["status"], "submitted");

        // Verify inbound message was published
        let msg = rx.recv().await.unwrap();
        assert_eq!(msg.channel, "http");
        assert_eq!(msg.sender_id, "a2a");
        assert_eq!(msg.content, "hello from A2A");
        assert_eq!(msg.chat_id, task_id);
    }

    #[tokio::test]
    async fn test_get_task_found() {
        let (state, _rx) = make_a2a_state();
        let task = A2aTask {
            id: "a2a-test-123".to_string(),
            status: TaskStatus::Completed,
            message: "original".to_string(),
            result: Some("done".to_string()),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:01:00Z".to_string(),
        };
        state.store.insert(task);

        let app = a2a_router(state);

        let req = Request::builder()
            .method("GET")
            .uri("/a2a/tasks/a2a-test-123")
            .body(Body::empty())
            .unwrap();

        let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"], "a2a-test-123");
        assert_eq!(json["status"], "completed");
        assert_eq!(json["result"], "done");
    }

    #[tokio::test]
    async fn test_get_task_not_found() {
        let (state, _rx) = make_a2a_state();
        let app = a2a_router(state);

        let req = Request::builder()
            .method("GET")
            .uri("/a2a/tasks/nonexistent")
            .body(Body::empty())
            .unwrap();

        let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "task not found");
    }

    #[test]
    fn test_task_store_evicts_oldest_at_capacity() {
        let store = A2aTaskStore::new();
        for i in 0..MAX_A2A_TASKS {
            store.insert(A2aTask {
                id: format!("task-{i}"),
                status: TaskStatus::Completed,
                message: String::new(),
                result: None,
                created_at: format!("{i:06}"),
                updated_at: String::new(),
            });
        }
        // Insert one more — oldest (task-0, created_at "000000") should be evicted
        store.insert(A2aTask {
            id: "task-new".to_string(),
            status: TaskStatus::Submitted,
            message: String::new(),
            result: None,
            created_at: format!("{:06}", MAX_A2A_TASKS),
            updated_at: String::new(),
        });
        assert!(store.get("task-new").is_some());
        assert!(store.get("task-0").is_none());
        let count = store.tasks.lock().unwrap().len();
        assert_eq!(count, MAX_A2A_TASKS);
    }

    #[tokio::test]
    async fn test_create_task_rejects_oversized_message() {
        let (state, _rx) = make_a2a_state();
        let app = a2a_router(state);

        let big_msg = "x".repeat(1_048_576 + 1); // super::super::MAX_MESSAGE_SIZE + 1
        let body_json = serde_json::json!({"message": big_msg});
        let req = Request::builder()
            .method("POST")
            .uri("/a2a/tasks")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body_json).unwrap()))
            .unwrap();

        let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn test_create_task_updates_store_to_working() {
        let (state, _rx) = make_a2a_state();
        let store = state.store.clone();
        let app = a2a_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/a2a/tasks")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"message": "check store"}"#))
            .unwrap();

        let resp: axum::http::Response<_> = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let task_id = json["task_id"].as_str().unwrap();

        // Task should exist in the store with Working status
        let task = store.get(task_id).unwrap();
        assert_eq!(task.status, TaskStatus::Working);
        assert_eq!(task.message, "check store");
    }
}
