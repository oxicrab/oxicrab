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
use std::path::PathBuf;
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
            .map(|m| {
                let mut map = HashMap::new();
                map.insert("role".to_string(), Value::String(m.role.clone()));
                map.insert("content".to_string(), Value::String(m.content.clone()));
                map
            })
            .collect()
    }

    pub fn get_full_history(&self) -> Vec<HashMap<String, Value>> {
        self.messages
            .iter()
            .map(|m| {
                let mut map = HashMap::new();
                map.insert("role".to_string(), Value::String(m.role.clone()));
                map.insert("content".to_string(), Value::String(m.content.clone()));
                map
            })
            .collect()
    }
}

pub struct SessionManager {
    _workspace: PathBuf,
    sessions_dir: PathBuf,
    cache: Mutex<LruCache<String, Session>>,
}

impl SessionManager {
    pub fn new(workspace: PathBuf) -> Result<Self> {
        let sessions_dir = ensure_dir(workspace.join("sessions"))?;
        Ok(Self {
            _workspace: workspace,
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
                if let Ok(mut cache) = self.cache.try_lock() {
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
                .write(true)
                .truncate(true)
                .open(&lock_path)
                .with_context(|| "Failed to open session lock file")?;
            lock_file
                .lock_exclusive()
                .with_context(|| "Failed to acquire exclusive lock on session file")?;
            atomic_write(&path_clone, &content).with_context(|| {
                format!("Failed to write session file: {}", path_clone.display())
            })?;
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
        }
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
}
