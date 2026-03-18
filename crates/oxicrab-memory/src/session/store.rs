use crate::session::Session;
use anyhow::Result;
use async_trait::async_trait;

/// Trait for session storage backends
/// This allows pluggable storage implementations (file-based, database, etc.)
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Get or create a session with the given key
    async fn get_or_create(&self, key: &str) -> Result<Session>;

    /// Save a session
    async fn save(&self, session: &Session) -> Result<()>;
}
