use crate::session::store::SessionStore;
use crate::utils::{ensure_dir, get_nanobot_home, safe_filename};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use tokio::sync::Mutex;

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
    pub fn new(key: String) -> Self {
        Self {
            key,
            messages: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    pub fn add_message(&mut self, role: String, content: String, extra: HashMap<String, Value>) {
        let msg = MessageData {
            role,
            content,
            timestamp: Utc::now().to_rfc3339(),
            extra,
        };
        self.messages.push(msg);
        self.updated_at = Utc::now();

        // Prune oldest messages
        if self.messages.len() > MAX_SESSION_MESSAGES {
            self.messages = self.messages[self.messages.len() - MAX_SESSION_MESSAGES..].to_vec();
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
        let sessions_dir = ensure_dir(get_nanobot_home()?.join("sessions"))?;
        Ok(Self {
            _workspace: workspace.clone(),
            sessions_dir,
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(MAX_CACHED_SESSIONS).unwrap(),
            )),
        })
    }

    fn get_session_path(&self, key: &str) -> PathBuf {
        let safe_key = safe_filename(&key.replace(":", "_"));
        self.sessions_dir.join(format!("{}.jsonl", safe_key))
    }

    pub async fn get_or_create(&self, key: &str) -> Result<Session> {
        // Check cache with single lock scope to prevent race conditions
        let cached_session = {
            let mut cache = self.cache.lock().await;
            cache.get(key).cloned()
        };

        if let Some(session) = cached_session {
            return Ok(session);
        }

        // Try to load from disk
        let session = self.load(key)?;
        let session = session.unwrap_or_else(|| Session::new(key.to_string()));

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

    fn load(&self, key: &str) -> Result<Option<Session>> {
        let path = self.get_session_path(key);
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read session file: {}", path.display()))?;

        let mut messages = Vec::new();
        let mut metadata = HashMap::new();
        let mut created_at = None;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let data: Value =
                serde_json::from_str(line).with_context(|| "Failed to parse session JSON line")?;

            if data.get("_type") == Some(&Value::String("metadata".to_string())) {
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
                for (k, v) in data.as_object().unwrap() {
                    if k != "role" && k != "content" && k != "timestamp" {
                        extra.insert(k.clone(), v.clone());
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
            messages = messages[messages.len() - MAX_SESSION_MESSAGES..].to_vec();
        }

        Ok(Some(Session {
            key: key.to_string(),
            messages,
            created_at: created_at.unwrap_or_else(Utc::now),
            updated_at: Utc::now(),
            metadata,
        }))
    }

    pub async fn save(&self, session: &Session) -> Result<()> {
        let path = self.get_session_path(&session.key);
        ensure_dir(path.parent().context("Session path has no parent")?)?;

        let mut content = String::new();

        // Write metadata line
        let metadata_line = serde_json::json!({
            "_type": "metadata",
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

        fs::write(&path, content)
            .with_context(|| format!("Failed to write session file: {}", path.display()))?;

        // Update cache
        {
            let mut cache = self.cache.lock().await;
            cache.put(session.key.clone(), session.clone());
        }

        Ok(())
    }

    pub async fn delete(&self, key: &str) -> Result<bool> {
        // Remove from cache
        {
            let mut cache = self.cache.lock().await;
            cache.pop(key);
        }

        // Remove file
        let path = self.get_session_path(key);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("Failed to delete session file: {}", path.display()))?;
            return Ok(true);
        }
        Ok(false)
    }

    fn list_sessions(&self) -> Result<Vec<HashMap<String, Value>>> {
        let mut sessions = Vec::new();

        for entry in fs::read_dir(&self.sessions_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension() != Some(std::ffi::OsStr::new("jsonl")) {
                continue;
            }

            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(first_line) = content.lines().next() {
                    if let Ok(data) = serde_json::from_str::<Value>(first_line) {
                        if data.get("_type") == Some(&Value::String("metadata".to_string())) {
                            let mut session_info = HashMap::new();
                            session_info.insert(
                                "key".to_string(),
                                Value::String(
                                    path.file_stem()
                                        .unwrap()
                                        .to_string_lossy()
                                        .replacen("_", ":", 1),
                                ),
                            );
                            if let Some(created_at) = data.get("created_at") {
                                session_info.insert("created_at".to_string(), created_at.clone());
                            }
                            if let Some(updated_at) = data.get("updated_at") {
                                session_info.insert("updated_at".to_string(), updated_at.clone());
                            }
                            session_info.insert(
                                "path".to_string(),
                                Value::String(path.to_string_lossy().to_string()),
                            );
                            sessions.push(session_info);
                        }
                    }
                }
            }
        }

        sessions.sort_by(|a, b| {
            let a_ts = a.get("updated_at").and_then(|v| v.as_str()).unwrap_or("");
            let b_ts = b.get("updated_at").and_then(|v| v.as_str()).unwrap_or("");
            b_ts.cmp(a_ts)
        });

        Ok(sessions)
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

    async fn list_sessions(&self) -> Result<Vec<HashMap<String, Value>>> {
        Ok(SessionManager::list_sessions(self)?)
    }
}
