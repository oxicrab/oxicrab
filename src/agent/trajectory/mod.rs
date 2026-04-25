//! Agent-side helpers for trajectory logging and cross-session skill
//! auto-save. The DB layer lives in `oxicrab-memory` (`MemoryDB`
//! `insert_trajectory_event` / `find_repeated_tool_sequences`).

use crate::agent::memory::memory_db::{MemoryDB, TrajectoryEvent, TrajectoryEventType};
use std::sync::Arc;
use tracing::warn;

pub mod skill_suggester;

#[cfg(test)]
mod tests;

/// Async-friendly logger: every method returns immediately (work runs
/// on `spawn_blocking` so the agent loop's hot path doesn't block on
/// SQLite). Errors are logged once and swallowed — failed observability
/// must never break user-facing replies.
pub struct TrajectoryLogger {
    db: Arc<MemoryDB>,
}

impl TrajectoryLogger {
    pub fn new(db: Arc<MemoryDB>) -> Self {
        Self { db }
    }

    /// Log one tool call. `tool` is the registered name; `action` is
    /// the action-based dispatch value if the tool has actions.
    pub fn log_tool_call(&self, session_id: &str, turn: u32, tool: &str, action: Option<&str>) {
        self.log(TrajectoryEvent {
            session_id: session_id.to_string(),
            turn_index: turn,
            event_type: TrajectoryEventType::ToolCall,
            tool_name: Some(tool.to_string()),
            action: action.map(str::to_string),
            latency_ms: None,
            is_error: None,
            created_at_ms: chrono::Utc::now().timestamp_millis(),
        });
    }

    /// Log a tool result, including its outcome flag.
    pub fn log_tool_result(
        &self,
        session_id: &str,
        turn: u32,
        tool: &str,
        action: Option<&str>,
        is_error: bool,
        latency_ms: i64,
    ) {
        self.log(TrajectoryEvent {
            session_id: session_id.to_string(),
            turn_index: turn,
            event_type: TrajectoryEventType::ToolResult,
            tool_name: Some(tool.to_string()),
            action: action.map(str::to_string),
            latency_ms: Some(latency_ms),
            is_error: Some(is_error),
            created_at_ms: chrono::Utc::now().timestamp_millis(),
        });
    }

    /// Mark the end of a turn — used by aggregation queries to bucket
    /// events.
    pub fn log_turn_end(&self, session_id: &str, turn: u32) {
        self.log(TrajectoryEvent {
            session_id: session_id.to_string(),
            turn_index: turn,
            event_type: TrajectoryEventType::TurnEnd,
            tool_name: None,
            action: None,
            latency_ms: None,
            is_error: None,
            created_at_ms: chrono::Utc::now().timestamp_millis(),
        });
    }

    fn log(&self, ev: TrajectoryEvent) {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = db.insert_trajectory_event(&ev) {
                warn!("trajectory: insert failed: {e}");
            }
        });
    }
}
