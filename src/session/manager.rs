use crate::session::store::SessionStore;
use crate::utils::{atomic_write, ensure_dir, safe_filename};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fs2::FileExt;
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
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
    sessions_dir: PathBuf,
    cache: Mutex<LruCache<String, Session>>,
}

impl SessionManager {
    pub fn new(workspace: &Path) -> Result<Self> {
        let sessions_dir = ensure_dir(workspace.join("sessions"))?;
        Ok(Self {
            sessions_dir,
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(MAX_CACHED_SESSIONS).expect("MAX_CACHED_SESSIONS must be > 0"),
            )),
        })
    }

    fn get_session_path(&self, key: &str) -> PathBuf {
        let safe_key = safe_filename(&key.replace(':', "_"));
        self.sessions_dir.join(format!("{}.jsonl", safe_key))
    }

    pub async fn get_or_create(&self, key: &str) -> Result<Session> {
        // Check cache with single lock scope to prevent race conditions
        let cached_session = {
            let mut cache = self.cache.lock().await;
            cache.get(key).cloned()
        };

        if let Some(session) = cached_session {
            debug!("session cache hit: {}", key);
            return Ok(session);
        }

        // Try to load from disk (in spawn_blocking to avoid blocking async runtime)
        let path = self.get_session_path(key);
        let loaded = tokio::task::spawn_blocking(move || Self::load_from_path(&path))
            .await
            .map_err(|e| anyhow::anyhow!("session load task failed: {}", e))??;
        let session = if let Some(s) = loaded {
            debug!("session loaded from disk: {}", key);
            s
        } else {
            debug!("session created: {}", key);
            Session::new(key.to_string())
        };

        // Put in cache - double-check pattern to avoid duplicates
        {
            let mut cache = self.cache.lock().await;
            // Check again in case another task loaded it
            if let Some(existing) = cache.get(key) {
                return Ok(existing.clone());
            }
            cache.put(key.to_string(), session.clone());
        }

        Ok(session)
    }

    /// Load a session from a file path. Extracted as a static method so it can
    /// be called from `spawn_blocking` without borrowing `self`.
    fn load_from_path(path: &std::path::Path) -> Result<Option<Session>> {
        if !path.exists() {
            return Ok(None);
        }

        let file = fs::File::open(path)
            .with_context(|| format!("Failed to open session file: {}", path.display()))?;
        file.lock_shared()
            .with_context(|| "Failed to acquire shared lock on session file")?;
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read session file: {}", path.display()))?;
        // lock released when `file` drops

        let mut messages = Vec::new();
        let mut metadata = HashMap::new();
        let mut created_at = None;

        // Derive session key from filename as fallback; prefer the key stored
        // in the metadata line (added in v0.11+) for round-trip fidelity.
        let fallback_key = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let mut key = fallback_key;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let data: Value =
                serde_json::from_str(line).with_context(|| "Failed to parse session JSON line")?;

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
                    .unwrap_or("")
                    .to_string();
                if role.is_empty() {
                    warn!("session message missing 'role' field, defaulting to empty");
                }
                let content = data
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let timestamp = data
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !timestamp.is_empty() && DateTime::parse_from_rfc3339(&timestamp).is_err() {
                    warn!(
                        "session message has invalid RFC3339 timestamp: {}",
                        timestamp
                    );
                }

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

        // Prune on load
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

    /// Delete session files older than `ttl_days` days.
    /// Runs once at startup to prevent unbounded disk accumulation.
    pub fn cleanup_old_sessions(&self, ttl_days: u32) -> Result<usize> {
        use std::time::{Duration, SystemTime};
        let cutoff = SystemTime::now() - Duration::from_secs(u64::from(ttl_days) * 86400);
        let mut deleted = 0;

        let entries = fs::read_dir(&self.sessions_dir).with_context(|| {
            format!(
                "Failed to read sessions dir: {}",
                self.sessions_dir.display()
            )
        })?;

        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
                continue;
            };
            if modified < cutoff {
                // Evict from in-memory cache before deleting from disk.
                // The cache is keyed by the original session key (e.g. "telegram:12345"),
                // but the filename is a safe_filename version of that key. We need to
                // try evicting by reconstructing the original key pattern. Since the
                // exact original key isn't stored in the filename, we iterate the cache
                // to find entries whose path matches this file.
                match self.cache.try_lock() {
                    Ok(mut cache) => {
                        // Collect keys to evict (can't mutate during iteration)
                        let keys_to_evict: Vec<String> = cache
                            .iter()
                            .filter(|(k, _)| self.get_session_path(k) == path)
                            .map(|(k, _)| k.clone())
                            .collect();
                        for k in keys_to_evict {
                            cache.pop(&k);
                        }
                    }
                    Err(_) => {
                        warn!(
                            "could not acquire cache lock during cleanup, skipping eviction for {}",
                            path.display()
                        );
                    }
                }
                if let Err(e) = fs::remove_file(&path) {
                    warn!("Failed to delete old session {}: {}", path.display(), e);
                } else {
                    info!("Cleaned up old session: {}", path.display());
                    deleted += 1;
                }
            }
        }

        if deleted > 0 {
            info!("Session cleanup: removed {} expired session(s)", deleted);
        }
        Ok(deleted)
    }

    pub async fn save(&self, session: &Session) -> Result<()> {
        let path = self.get_session_path(&session.key);

        let mut content = String::new();

        // Write metadata line (includes original key for round-trip fidelity)
        let metadata_line = serde_json::json!({
            "_type": "metadata",
            "key": session.key,
            "created_at": session.created_at.to_rfc3339(),
            "updated_at": session.updated_at.to_rfc3339(),
            "metadata": session.metadata,
        });
        content.push_str(&serde_json::to_string(&metadata_line)?);
        content.push('\n');

        // Write messages
        for msg in &session.messages {
            let mut msg_obj = serde_json::json!({
                "role": msg.role,
                "content": msg.content,
                "timestamp": msg.timestamp,
            });
            for (k, v) in &msg.extra {
                msg_obj[k] = v.clone();
            }
            content.push_str(&serde_json::to_string(&msg_obj)?);
            content.push('\n');
        }

        let session_key = session.key.clone();
        let msg_count = session.messages.len();

        // Perform blocking file I/O (locking + atomic write) off the async runtime
        let path_clone = path.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            ensure_dir(path_clone.parent().context("Session path has no parent")?)?;
            let lock_path = path_clone.with_extension("jsonl.lock");
            let lock_file = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&lock_path)
                .with_context(|| "Failed to open session lock file")?;
            lock_file
                .lock_exclusive()
                .with_context(|| "Failed to acquire exclusive lock on session file")?;
            atomic_write(&path_clone, &content).with_context(|| {
                format!("Failed to write session file: {}", path_clone.display())
            })?;
            // Clean up the lock file — it's only needed during the write
            let _ = std::fs::remove_file(&lock_path);
            Ok(())
        })
        .await??;

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
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn add_message_never_exceeds_max(count in 0..500usize) {
            let mut session = Session::new("prop:test".to_string());
            let extra = HashMap::new();
            for i in 0..count {
                session.add_message("user", format!("msg {}", i), extra.clone());
            }
            prop_assert!(session.messages.len() <= MAX_SESSION_MESSAGES);
        }

        #[test]
        fn get_history_respects_limit(
            msg_count in 0..300usize,
            limit in 0..400usize,
        ) {
            let mut session = Session::new("prop:history".to_string());
            let extra = HashMap::new();
            for i in 0..msg_count {
                session.add_message("user", format!("msg {}", i), extra.clone());
            }
            let history = session.get_history(limit);
            prop_assert!(history.len() <= limit);
            prop_assert!(history.len() <= session.messages.len());
        }
    }

    #[test]
    fn test_session_get_history_with_limit() {
        let mut session = Session::new("test_key".to_string());
        let extra = HashMap::new();

        // Add 5 messages
        for i in 0..5 {
            session.add_message("user".to_string(), format!("Message {}", i), extra.clone());
        }

        let history = session.get_history(3);
        assert_eq!(history.len(), 3);

        // Should return last 3 messages (indices 2, 3, 4)
        assert_eq!(
            history[0]["content"],
            Value::String("Message 2".to_string())
        );
        assert_eq!(
            history[1]["content"],
            Value::String("Message 3".to_string())
        );
        assert_eq!(
            history[2]["content"],
            Value::String("Message 4".to_string())
        );
    }

    #[test]
    fn test_session_get_full_history() {
        let mut session = Session::new("test_key".to_string());
        let extra = HashMap::new();

        // Add 3 messages
        for i in 0..3 {
            session.add_message("user".to_string(), format!("Message {}", i), extra.clone());
        }

        let history = session.get_full_history();
        assert_eq!(history.len(), 3);

        for (i, entry) in history.iter().enumerate() {
            assert_eq!(entry["content"], Value::String(format!("Message {}", i)));
            assert_eq!(entry["role"], Value::String("user".to_string()));
            // Timestamp and extra fields are now included
            assert!(entry.contains_key("timestamp"));
        }
    }

    #[test]
    fn test_session_get_history_includes_extra_fields() {
        let mut session = Session::new("test_key".to_string());
        let mut extra = HashMap::new();
        extra.insert(
            "tool_call_id".to_string(),
            Value::String("tc_123".to_string()),
        );
        session.add_message("tool", "result", extra);

        let history = session.get_history(10);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["role"], Value::String("tool".to_string()));
        assert_eq!(
            history[0]["tool_call_id"],
            Value::String("tc_123".to_string())
        );
        assert!(history[0].contains_key("timestamp"));
    }

    #[test]
    fn test_session_add_message_prunes_at_capacity() {
        let mut session = Session::new("test_key".to_string());
        let extra = HashMap::new();

        // Add MAX_SESSION_MESSAGES + 5 messages
        for i in 0..(MAX_SESSION_MESSAGES + 5) {
            session.add_message("user".to_string(), format!("Message {}", i), extra.clone());
        }

        // Should be capped at MAX_SESSION_MESSAGES
        assert_eq!(session.messages.len(), MAX_SESSION_MESSAGES);

        // First message should be the one at index 5 (0-4 should be pruned)
        assert_eq!(session.messages[0].content, "Message 5");

        // Last message should be the last one we added
        assert_eq!(
            session.messages[MAX_SESSION_MESSAGES - 1].content,
            format!("Message {}", MAX_SESSION_MESSAGES + 4)
        );
    }

    // --- SessionManager persistence tests ---

    #[tokio::test]
    async fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path()).unwrap();

        let mut session = Session::new("roundtrip:key");
        session.add_message("user", "hello world", HashMap::new());
        session.add_message("assistant", "hi there", HashMap::new());
        mgr.save(&session).await.unwrap();

        // Create a fresh manager at the same path — forces disk load
        let mgr2 = SessionManager::new(dir.path()).unwrap();
        let loaded = mgr2.get_or_create("roundtrip:key").await.unwrap();

        assert_eq!(loaded.key, "roundtrip:key");
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].role, "user");
        assert_eq!(loaded.messages[0].content, "hello world");
        assert_eq!(loaded.messages[1].role, "assistant");
        assert_eq!(loaded.messages[1].content, "hi there");
    }

    #[tokio::test]
    async fn test_cache_hit_returns_same_session() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path()).unwrap();

        // First call creates and caches
        let s1 = mgr.get_or_create("cache:test").await.unwrap();
        assert_eq!(s1.key, "cache:test");

        // Second call should return from cache (same content)
        let s2 = mgr.get_or_create("cache:test").await.unwrap();
        assert_eq!(s2.key, s1.key);
        assert_eq!(s2.messages.len(), s1.messages.len());
    }

    #[tokio::test]
    async fn test_lru_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path()).unwrap();

        // Save a session with known content
        let mut first = Session::new("evict:first");
        first.add_message("user", "important data", HashMap::new());
        mgr.save(&first).await.unwrap();

        // Fill cache to capacity with other sessions to evict the first one
        for i in 0..MAX_CACHED_SESSIONS {
            let _ = mgr
                .get_or_create(&format!("evict:filler_{}", i))
                .await
                .unwrap();
        }

        // "evict:first" should be evicted from LRU cache.
        // get_or_create should still load it from disk.
        let reloaded = mgr.get_or_create("evict:first").await.unwrap();
        assert_eq!(reloaded.key, "evict:first");
        assert_eq!(reloaded.messages.len(), 1);
        assert_eq!(reloaded.messages[0].content, "important data");
    }

    #[tokio::test]
    async fn test_cleanup_old_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path()).unwrap();

        // Save a session so a file exists
        let session = Session::new("old:session");
        mgr.save(&session).await.unwrap();

        let path = mgr.get_session_path("old:session");
        assert!(path.exists());

        // Backdate the file to 100 days ago
        let old_time =
            filetime::FileTime::from_unix_time(chrono::Utc::now().timestamp() - 100 * 86400, 0);
        filetime::set_file_mtime(&path, old_time).unwrap();

        // Cleanup with 30-day TTL should delete it
        let deleted = mgr.cleanup_old_sessions(30).unwrap();
        assert_eq!(deleted, 1);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn test_cleanup_preserves_recent_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path()).unwrap();

        let session = Session::new("recent:session");
        mgr.save(&session).await.unwrap();

        let path = mgr.get_session_path("recent:session");
        assert!(path.exists());

        // Cleanup with 30-day TTL should preserve a just-created file
        let deleted = mgr.cleanup_old_sessions(30).unwrap();
        assert_eq!(deleted, 0);
        assert!(path.exists());
    }

    #[tokio::test]
    async fn test_load_handles_metadata_line() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path()).unwrap();

        // Save a session (which writes a metadata line with key + created_at)
        let mut session = Session::new("meta:test");
        session.metadata.insert(
            "custom".to_string(),
            serde_json::Value::String("value".to_string()),
        );
        mgr.save(&session).await.unwrap();

        // Load from a fresh manager
        let mgr2 = SessionManager::new(dir.path()).unwrap();
        let loaded = mgr2.get_or_create("meta:test").await.unwrap();

        assert_eq!(loaded.key, "meta:test");
        assert_eq!(
            loaded.metadata.get("custom"),
            Some(&serde_json::Value::String("value".to_string()))
        );
    }

    #[tokio::test]
    async fn test_load_handles_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path()).unwrap();

        // No file on disk — should create a new empty session
        let session = mgr.get_or_create("does:not:exist").await.unwrap();
        assert_eq!(session.key, "does:not:exist");
        assert!(session.messages.is_empty());
    }

    #[tokio::test]
    async fn test_session_key_roundtrip_with_colons() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path()).unwrap();

        // Keys with colons are common (e.g. "telegram:12345")
        let mut session = Session::new("telegram:12345");
        session.add_message("user", "test", HashMap::new());
        mgr.save(&session).await.unwrap();

        let mgr2 = SessionManager::new(dir.path()).unwrap();
        let loaded = mgr2.get_or_create("telegram:12345").await.unwrap();

        // The metadata line preserves the original key despite filename mangling
        assert_eq!(loaded.key, "telegram:12345");
        assert_eq!(loaded.messages.len(), 1);
    }
}
