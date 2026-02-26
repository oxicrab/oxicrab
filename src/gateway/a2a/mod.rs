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
mod tests;
