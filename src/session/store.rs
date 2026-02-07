use crate::session::Session;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use serde_json::Value;

/// Trait for session storage backends
/// This allows pluggable storage implementations (file-based, database, etc.)
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Get or create a session with the given key
    async fn get_or_create(&self, key: &str) -> Result<Session>;

    /// Save a session
    async fn save(&self, session: &Session) -> Result<()>;

    /// Delete a session
    async fn delete(&self, key: &str) -> Result<bool>;

    /// List all sessions
    async fn list_sessions(&self) -> Result<Vec<HashMap<String, Value>>>;
}
