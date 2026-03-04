use crate::agent::memory::MemoryDB;
use crate::session::store::SessionStore;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

const MAX_CACHED_SESSIONS: usize = 64;
const MAX_SESSION_MESSAGES: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub key: String,
    pub messages: Vec<MessageData>,
    #[serde(default = "chrono::Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "chrono::Utc::now")]
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageData {
    pub role: String,
    pub content: String,
    pub timestamp: String,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl Session {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            messages: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    pub fn add_message(
        &mut self,
        role: impl Into<String>,
        content: impl Into<String>,
        extra: HashMap<String, Value>,
    ) {
        let msg = MessageData {
            role: role.into(),
            content: content.into(),
            timestamp: Utc::now().to_rfc3339(),
            extra,
        };
        self.messages.push(msg);
        self.updated_at = Utc::now();

        // Prune oldest messages
        if self.messages.len() > MAX_SESSION_MESSAGES {
            let drain_count = self.messages.len() - MAX_SESSION_MESSAGES;
            self.messages.drain(..drain_count);
        }
    }

    pub fn get_history(&self, max_messages: usize) -> Vec<HashMap<String, Value>> {
        let start = if self.messages.len() > max_messages {
            self.messages.len() - max_messages
        } else {
            0
        };

        self.messages[start..]
            .iter()
            .map(Self::message_to_map)
            .collect()
    }

    pub fn get_full_history(&self) -> Vec<HashMap<String, Value>> {
        self.messages.iter().map(Self::message_to_map).collect()
    }

    fn message_to_map(m: &MessageData) -> HashMap<String, Value> {
        let mut map = HashMap::new();
        map.insert("role".to_string(), Value::String(m.role.clone()));
        map.insert("content".to_string(), Value::String(m.content.clone()));
        if !m.timestamp.is_empty() {
            map.insert("timestamp".to_string(), Value::String(m.timestamp.clone()));
        }
        for (k, v) in &m.extra {
            map.insert(k.clone(), v.clone());
        }
        map
    }
}

pub struct SessionManager {
    db: Arc<MemoryDB>,
    cache: Mutex<LruCache<String, Session>>,
}

impl SessionManager {
    pub fn new(workspace: &Path) -> Result<Self> {
        let memory_dir = workspace.join("memory");
        std::fs::create_dir_all(&memory_dir).with_context(|| {
            format!(
                "failed to create memory directory: {}",
                memory_dir.display()
            )
        })?;
        let db_path = memory_dir.join("memory.sqlite3");
        let db = Arc::new(MemoryDB::new(db_path)?);
        let mgr = Self {
            db,
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(MAX_CACHED_SESSIONS).expect("MAX_CACHED_SESSIONS must be > 0"),
            )),
        };

        // Migrate existing JSONL files on first use
        let sessions_dir = workspace.join("sessions");
        if sessions_dir.is_dir()
            && let Err(e) = mgr.migrate_jsonl_files(&sessions_dir)
        {
            warn!("session migration from JSONL failed: {e}");
        }

        Ok(mgr)
    }

    /// Create a `SessionManager` from an existing `MemoryDB` instance.
    /// Used when the agent loop already has a db reference.
    pub fn with_db(db: Arc<MemoryDB>) -> Self {
        Self {
            db,
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(MAX_CACHED_SESSIONS).expect("MAX_CACHED_SESSIONS must be > 0"),
            )),
        }
    }

    /// Migrate existing JSONL session files into `SQLite`.
    /// Runs once; after migration, the `sessions/` directory is renamed to
    /// `sessions.migrated/` to prevent re-migration.
    fn migrate_jsonl_files(&self, sessions_dir: &Path) -> Result<()> {
        let entries: Vec<_> = std::fs::read_dir(sessions_dir)?
            .filter_map(std::result::Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "jsonl"))
            .collect();

        if entries.is_empty() {
            return Ok(());
        }

        info!("migrating {} JSONL session files to SQLite", entries.len());

        for entry in &entries {
            let path = entry.path();
            match Self::load_jsonl_file(&path) {
                Ok(Some(session)) => {
                    let data = serde_json::to_string(&session)?;
                    if let Err(e) = self.db.save_session(&session.key, &data) {
                        warn!("failed to migrate session {}: {e}", session.key);
                    }
                }
                Ok(None) => {}
                Err(e) => warn!("failed to read session file {}: {e}", path.display()),
            }
        }

        // Rename the old directory to mark migration complete
        let migrated_dir = sessions_dir.with_file_name("sessions.migrated");
        if let Err(e) = std::fs::rename(sessions_dir, &migrated_dir) {
            warn!(
                "could not rename sessions dir after migration: {e}; files will be re-migrated next time"
            );
        } else {
            info!(
                "session migration complete; old files moved to {}",
                migrated_dir.display()
            );
        }

        Ok(())
    }

    /// Load a session from a JSONL file (for migration).
    fn load_jsonl_file(path: &std::path::Path) -> Result<Option<Session>> {
        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read session file: {}", path.display()))?;

        let mut messages = Vec::new();
        let mut metadata = HashMap::new();
        let mut created_at = None;

        let fallback_key = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let mut key = fallback_key;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let data: Value =
                serde_json::from_str(line).with_context(|| "failed to parse session JSON line")?;

            if data.get("_type") == Some(&Value::String("metadata".to_string())) {
                if let Some(stored_key) = data.get("key").and_then(|v| v.as_str()) {
                    key = stored_key.to_string();
                }
                if let Some(meta) = data.get("metadata").and_then(|v| v.as_object()) {
                    for (k, v) in meta {
                        metadata.insert(k.clone(), v.clone());
                    }
                }
                if let Some(ts) = data.get("created_at").and_then(|v| v.as_str()) {
                    created_at = DateTime::parse_from_rfc3339(ts)
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc));
                }
            } else {
                let role = data
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let content = data
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let timestamp = data
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();

                let mut extra = HashMap::new();
                if let Some(obj) = data.as_object() {
                    for (k, v) in obj {
                        if k != "role" && k != "content" && k != "timestamp" && k != "_type" {
                            extra.insert(k.clone(), v.clone());
                        }
                    }
                }

                messages.push(MessageData {
                    role,
                    content,
                    timestamp,
                    extra,
                });
            }
        }

        if messages.len() > MAX_SESSION_MESSAGES {
            let drain_count = messages.len() - MAX_SESSION_MESSAGES;
            messages.drain(..drain_count);
        }

        Ok(Some(Session {
            key,
            messages,
            created_at: created_at.unwrap_or_else(Utc::now),
            updated_at: Utc::now(),
            metadata,
        }))
    }

    pub async fn get_or_create(&self, key: &str) -> Result<Session> {
        // Check cache
        let cached_session = {
            let mut cache = self.cache.lock().await;
            cache.get(key).cloned()
        };

        if let Some(session) = cached_session {
            debug!("session cache hit: {}", key);
            return Ok(session);
        }

        // Try to load from SQLite
        let db = self.db.clone();
        let key_owned = key.to_string();
        let loaded = tokio::task::spawn_blocking(move || db.load_session(&key_owned))
            .await
            .map_err(|e| anyhow::anyhow!("session load task failed: {e}"))??;

        let session = if let Some(data) = loaded {
            let mut s: Session = serde_json::from_str(&data)
                .with_context(|| "failed to parse session JSON from database")?;
            // Ensure key matches (migration may have stored under a different key)
            s.key = key.to_string();
            debug!("session loaded from database: {}", key);
            s
        } else {
            debug!("session created: {}", key);
            Session::new(key.to_string())
        };

        // Put in cache (double-check to avoid duplicates)
        {
            let mut cache = self.cache.lock().await;
            if let Some(existing) = cache.get(key) {
                return Ok(existing.clone());
            }
            cache.put(key.to_string(), session.clone());
        }

        Ok(session)
    }

    /// Delete session files older than `ttl_days` days.
    pub fn cleanup_old_sessions(&self, ttl_days: u32) -> Result<usize> {
        let deleted = self.db.cleanup_sessions(ttl_days)?;
        if deleted > 0 {
            info!("session cleanup: removed {} expired session(s)", deleted);
        }
        Ok(deleted)
    }

    pub async fn save(&self, session: &Session) -> Result<()> {
        let data = serde_json::to_string(session).context("failed to serialize session to JSON")?;
        let session_key = session.key.clone();
        let msg_count = session.messages.len();

        let db = self.db.clone();
        let key = session.key.clone();
        tokio::task::spawn_blocking(move || db.save_session(&key, &data))
            .await
            .map_err(|e| anyhow::anyhow!("session save task failed: {e}"))??;

        debug!("session saved: {} ({} messages)", session_key, msg_count);

        // Update cache
        {
            let mut cache = self.cache.lock().await;
            cache.put(session.key.clone(), session.clone());
        }

        Ok(())
    }
}

#[async_trait]
impl SessionStore for SessionManager {
    async fn get_or_create(&self, key: &str) -> Result<Session> {
        SessionManager::get_or_create(self, key).await
    }

    async fn save(&self, session: &Session) -> Result<()> {
        SessionManager::save(self, session).await
    }
}

#[cfg(test)]
mod tests;
