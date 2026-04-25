use crate::memory_db::MemoryDB;
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
        let mut extra = extra;

        // Cap reasoning_content to prevent multi-MB session bloat
        const MAX_REASONING_CHARS: usize = 2000;
        if let Some(Value::String(rc)) = extra.get("reasoning_content")
            && rc.len() > MAX_REASONING_CHARS
        {
            extra.insert(
                "reasoning_content".to_string(),
                Value::String(format!(
                    "{}...[truncated]",
                    &rc[..rc.floor_char_boundary(MAX_REASONING_CHARS)]
                )),
            );
        }

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
        const RESERVED_KEYS: &[&str] = &["role", "content", "timestamp"];

        let mut map = HashMap::new();
        map.insert("role".to_string(), Value::String(m.role.clone()));
        map.insert("content".to_string(), Value::String(m.content.clone()));
        if !m.timestamp.is_empty() {
            map.insert("timestamp".to_string(), Value::String(m.timestamp.clone()));
        }
        for (k, v) in &m.extra {
            if !RESERVED_KEYS.contains(&k.as_str()) {
                map.insert(k.clone(), v.clone());
            }
        }
        map
    }
}

pub struct SessionManager {
    db: Arc<MemoryDB>,
    cache: Mutex<LruCache<String, Session>>,
    /// Per-key serialization for `get_or_create`, `delete`, and
    /// `rotate_session` so concurrent operations on the same key cannot
    /// produce split-brain sessions or resurrect a just-deleted row.
    key_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
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
            key_locks: Mutex::new(HashMap::new()),
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
            key_locks: Mutex::new(HashMap::new()),
        }
    }

    /// Get (or insert) the per-key write lock for `key`. The lock map
    /// itself is short-lived: callers should drop the outer guard before
    /// awaiting the per-key lock to avoid blocking unrelated keys.
    async fn key_lock(&self, key: &str) -> Arc<Mutex<()>> {
        let mut map = self.key_locks.lock().await;
        if let Some(existing) = map.get(key) {
            return existing.clone();
        }
        let new_lock = Arc::new(Mutex::new(()));
        map.insert(key.to_string(), new_lock.clone());
        new_lock
    }

    /// Drop the per-key lock entry when no other holder exists. Called on
    /// the slow path after a successful `delete` so the map doesn't grow
    /// unboundedly across short-lived sessions.
    async fn maybe_evict_key_lock(&self, key: &str) {
        let mut map = self.key_locks.lock().await;
        if let Some(existing) = map.get(key)
            && Arc::strong_count(existing) == 1
        {
            map.remove(key);
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
        // Cache-only fast path: a hit needs no per-key locking because
        // the cache is itself a tokio mutex and the session is cloned.
        let cached_session = {
            let mut cache = self.cache.lock().await;
            cache.get(key).cloned()
        };
        if let Some(session) = cached_session
            && !self.should_rotate(&session)
        {
            debug!("session cache hit: {}", key);
            return Ok(session);
        }

        // Slow path: serialize on the per-key lock so a concurrent
        // `delete` cannot resurrect a deleted row in cache, and two
        // racing rotations cannot produce split-brain Session objects.
        let lock = self.key_lock(key).await;
        let _key_guard = lock.lock().await;

        // Re-check the cache under the per-key lock. Another caller may
        // have populated/rotated it while we were waiting.
        let cached_session = {
            let mut cache = self.cache.lock().await;
            cache.get(key).cloned()
        };
        if let Some(session) = cached_session {
            if self.should_rotate(&session) {
                debug!("session daily rotation: {}", key);
                return self.rotate_session_locked(key).await;
            }
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

            if self.should_rotate(&s) {
                debug!("session daily rotation: {}", key);
                return self.rotate_session_locked(key).await;
            }

            debug!("session loaded from database: {}", key);
            s
        } else {
            debug!("session created: {}", key);
            Session::new(key.to_string())
        };

        {
            let mut cache = self.cache.lock().await;
            cache.put(key.to_string(), session.clone());
        }

        Ok(session)
    }

    /// Check whether a session should be rotated (created on a previous UTC day).
    fn should_rotate(&self, session: &Session) -> bool {
        session.created_at.date_naive() < Utc::now().date_naive()
    }

    /// Delete the old session and return a fresh one. **Caller must hold
    /// the per-key lock** — used by `get_or_create` after it acquires the
    /// lock on the slow path. Public callers go through `rotate_session`.
    async fn rotate_session_locked(&self, key: &str) -> Result<Session> {
        info!("rotating session: {}", key);
        let db = self.db.clone();
        let key_owned = key.to_string();
        tokio::task::spawn_blocking(move || db.delete_session(&key_owned))
            .await
            .map_err(|e| anyhow::anyhow!("session delete task failed: {e}"))??;

        let session = Session::new(key.to_string());
        let mut cache = self.cache.lock().await;
        cache.put(key.to_string(), session.clone());
        Ok(session)
    }

    /// Delete sessions older than `ttl_days` days from the database.
    /// Selectively evicts deleted sessions from the LRU cache.
    pub async fn cleanup_old_sessions(&self, ttl_days: u32) -> Result<usize> {
        let deleted_keys = self.db.cleanup_sessions(ttl_days)?;
        let count = deleted_keys.len();
        if count > 0 {
            info!("session cleanup: removed {} expired session(s)", count);
            let mut cache = self.cache.lock().await;
            for key in &deleted_keys {
                cache.pop(key);
            }
        }
        Ok(count)
    }

    /// Delete a session from both the database and cache.
    pub async fn delete(&self, key: &str) -> Result<bool> {
        // Hold the per-key lock across the DB delete and cache pop so a
        // concurrent `get_or_create` can't observe a deleted row and put
        // a stale clone back in cache.
        let lock = self.key_lock(key).await;
        let _key_guard = lock.lock().await;

        let db = self.db.clone();
        let key_owned = key.to_string();
        let existed = tokio::task::spawn_blocking(move || db.delete_session(&key_owned))
            .await
            .map_err(|e| anyhow::anyhow!("session delete task failed: {e}"))??;

        {
            let mut cache = self.cache.lock().await;
            cache.pop(key);
        }

        if existed {
            info!("session deleted: {}", key);
        }
        // Best-effort: prune the per-key lock entry once the last
        // outstanding handle (this one) is dropped.
        drop(_key_guard);
        self.maybe_evict_key_lock(key).await;
        Ok(existed)
    }

    pub async fn save(&self, session: &Session) -> Result<()> {
        let session_key = session.key.clone();

        // Serialize + write inside spawn_blocking so neither JSON serialization
        // (can be expensive for large sessions) nor SQLite I/O blocks the
        // async runtime.
        let db = self.db.clone();
        let key = session.key.clone();
        let session_clone = session.clone();
        tokio::task::spawn_blocking(move || {
            let data = serde_json::to_string(&session_clone)
                .context("failed to serialize session to JSON")?;
            db.save_session(&key, &data)
        })
        .await
        .map_err(|e| anyhow::anyhow!("session save task failed: {e}"))??;

        debug!("session saved: key={}", session_key);
        metrics::counter!("oxicrab_sessions_saved_total").increment(1);

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

    async fn delete(&self, key: &str) -> Result<bool> {
        SessionManager::delete(self, key).await
    }
}

#[cfg(test)]
mod tests;
