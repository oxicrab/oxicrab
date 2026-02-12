use super::client::ObsidianApiClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Metadata for a single cached file.
#[derive(Clone, Serialize, Deserialize)]
pub struct CachedFileMeta {
    pub content_hash: String,
    pub last_synced_at: i64,
    pub size: u64,
}

/// Persistent sync state for the vault cache.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct SyncState {
    pub files: HashMap<String, CachedFileMeta>,
    pub last_full_sync_at: i64,
}

/// A queued write operation for when the API is unreachable.
#[derive(Clone, Serialize, Deserialize)]
pub struct QueuedWrite {
    pub path: String,
    pub content: String,
    pub operation: String, // "write" or "append"
    pub queued_at: i64,
    pub pre_write_hash: Option<String>,
}

pub(crate) fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

/// Sanitize vault name for use as a directory name.
pub(crate) fn safe_vault_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Local file cache for an Obsidian vault, backed by the REST API.
pub struct ObsidianCache {
    cache_dir: PathBuf,
    state_path: PathBuf,
    queue_path: PathBuf,
    pub(crate) client: Arc<ObsidianApiClient>,
    pub(crate) state: Arc<Mutex<SyncState>>,
    pub(crate) write_queue: Arc<Mutex<Vec<QueuedWrite>>>,
}

impl ObsidianCache {
    pub async fn new(client: Arc<ObsidianApiClient>, vault_name: &str) -> Result<Self> {
        let home =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
        let base = home
            .join(".nanobot")
            .join("obsidian_cache")
            .join(safe_vault_name(vault_name));
        std::fs::create_dir_all(&base)?;

        let state_path = base.join("sync_state.json");
        let queue_path = base.join("write_queue.json");

        let state = if state_path.exists() {
            let data = std::fs::read_to_string(&state_path)?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            SyncState::default()
        };

        let write_queue = if queue_path.exists() {
            let data = std::fs::read_to_string(&queue_path)?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(Self {
            cache_dir: base,
            state_path,
            queue_path,
            client,
            state: Arc::new(Mutex::new(state)),
            write_queue: Arc::new(Mutex::new(write_queue)),
        })
    }

    #[cfg(test)]
    pub fn with_dir(client: Arc<ObsidianApiClient>, cache_dir: PathBuf) -> Self {
        let state_path = cache_dir.join("sync_state.json");
        let queue_path = cache_dir.join("write_queue.json");
        Self {
            cache_dir,
            state_path,
            queue_path,
            client,
            state: Arc::new(Mutex::new(SyncState::default())),
            write_queue: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Read a file from the local cache.
    pub async fn read_cached(&self, path: &str) -> Option<String> {
        let file_path = self.cache_dir.join(path);
        std::fs::read_to_string(file_path).ok()
    }

    /// List cached files, optionally filtered by folder prefix.
    pub async fn list_cached(&self, folder: Option<&str>) -> Vec<String> {
        let state = self.state.lock().await;
        let mut files: Vec<String> = state
            .files
            .keys()
            .filter(|k| {
                if let Some(prefix) = folder {
                    k.starts_with(prefix)
                } else {
                    true
                }
            })
            .cloned()
            .collect();
        files.sort();
        files
    }

    /// Case-insensitive full-text search across cached files.
    pub async fn search_cached(&self, query: &str) -> Vec<(String, String)> {
        let state = self.state.lock().await;
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for path in state.files.keys() {
            let file_path = self.cache_dir.join(path);
            if let Ok(content) = std::fs::read_to_string(&file_path) {
                for line in content.lines() {
                    if line.to_lowercase().contains(&query_lower) {
                        results.push((path.clone(), line.to_string()));
                        if results.len() >= 50 {
                            return results;
                        }
                    }
                }
            }
        }

        results
    }

    /// Write a file. If online, write through to API + cache. If offline, queue.
    pub async fn write_file(&self, path: &str, content: &str) -> Result<String> {
        // Update cache optimistically
        self.write_to_cache(path, content)?;
        self.update_state_entry(path, content).await;

        if self.client.is_reachable().await {
            match self.client.write_file(path, content).await {
                Ok(()) => Ok(format!("Written to '{}'.", path)),
                Err(e) => {
                    warn!("API write failed, queueing: {}", e);
                    self.enqueue_write(path, content, "write").await?;
                    Ok(format!(
                        "Written to cache. API unreachable — queued for sync: {}",
                        path
                    ))
                }
            }
        } else {
            self.enqueue_write(path, content, "write").await?;
            Ok(format!(
                "API unreachable. Written to cache and queued for sync: {}",
                path
            ))
        }
    }

    /// Append to a file. If online, append via API + update cache. If offline, queue.
    pub async fn append_file(&self, path: &str, content: &str) -> Result<String> {
        // Build full content from cache + append
        let existing = self.read_cached(path).await.unwrap_or_default();
        let full_content = format!("{}{}", existing, content);

        // Update cache optimistically
        self.write_to_cache(path, &full_content)?;
        self.update_state_entry(path, &full_content).await;

        if self.client.is_reachable().await {
            match self.client.append_file(path, content).await {
                Ok(()) => Ok(format!("Appended to '{}'.", path)),
                Err(e) => {
                    warn!("API append failed, queueing: {}", e);
                    self.enqueue_write(path, content, "append").await?;
                    Ok(format!(
                        "Appended to cache. API unreachable — queued for sync: {}",
                        path
                    ))
                }
            }
        } else {
            self.enqueue_write(path, content, "append").await?;
            Ok(format!(
                "API unreachable. Appended to cache and queued for sync: {}",
                path
            ))
        }
    }

    /// Flush the write queue, pushing queued writes to the API.
    pub async fn flush_write_queue(&self) -> Result<()> {
        let mut queue = self.write_queue.lock().await;
        if queue.is_empty() {
            return Ok(());
        }

        info!("Flushing {} queued writes", queue.len());
        let mut remaining = Vec::new();

        for item in queue.drain(..) {
            // Check for conflict: if the remote file changed since we queued the write
            if let Some(ref pre_hash) = item.pre_write_hash {
                match self.client.read_file(&item.path).await {
                    Ok(remote_content) => {
                        let remote_hash = content_hash(&remote_content);
                        if &remote_hash != pre_hash {
                            // Conflict — save remote version as .conflict.md
                            let conflict_path = format!(
                                "{}.conflict.md",
                                item.path.strip_suffix(".md").unwrap_or(&item.path)
                            );
                            warn!(
                                "Conflict detected for '{}', saving remote as '{}'",
                                item.path, conflict_path
                            );
                            let _ = self.write_to_cache(&conflict_path, &remote_content);
                        }
                    }
                    Err(_) => {
                        // File doesn't exist remotely yet — no conflict
                    }
                }
            }

            let result = match item.operation.as_str() {
                "append" => self.client.append_file(&item.path, &item.content).await,
                _ => self.client.write_file(&item.path, &item.content).await,
            };

            if let Err(e) = result {
                warn!("Failed to flush write for '{}': {}", item.path, e);
                remaining.push(item);
            } else {
                debug!("Flushed queued write: {}", item.path);
            }
        }

        *queue = remaining;
        drop(queue);
        self.persist_queue().await?;
        Ok(())
    }

    /// Full sync: list remote files, download new/changed, remove deleted.
    pub async fn full_sync(&self) -> Result<()> {
        let remote_files = self.client.list_files().await?;
        let remote_set: std::collections::HashSet<&str> =
            remote_files.iter().map(|s| s.as_str()).collect();

        let mut state = self.state.lock().await;
        let mut updated = 0u32;
        let mut added = 0u32;
        let mut removed = 0u32;

        // Download new/changed files
        for file_path in &remote_files {
            let needs_download = match state.files.get(file_path) {
                Some(meta) => {
                    // Re-download if file is older than 1 sync interval
                    // We can't compare hashes without downloading, so just check
                    // if the file exists in cache
                    !self.cache_dir.join(file_path).exists()
                        || meta.last_synced_at < state.last_full_sync_at
                }
                None => true,
            };

            if needs_download {
                match self.client.read_file(file_path).await {
                    Ok(content) => {
                        let hash = content_hash(&content);
                        let existing_hash =
                            state.files.get(file_path).map(|m| m.content_hash.as_str());

                        if existing_hash != Some(&hash) {
                            if let Err(e) = self.write_to_cache(file_path, &content) {
                                warn!("Failed to cache '{}': {}", file_path, e);
                                continue;
                            }
                            if state.files.contains_key(file_path) {
                                updated += 1;
                            } else {
                                added += 1;
                            }
                        }

                        let now = chrono::Utc::now().timestamp();
                        state.files.insert(
                            file_path.clone(),
                            CachedFileMeta {
                                content_hash: hash,
                                last_synced_at: now,
                                size: content.len() as u64,
                            },
                        );
                    }
                    Err(e) => {
                        warn!("Failed to download '{}': {}", file_path, e);
                    }
                }
            }
        }

        // Remove locally cached files that were deleted remotely
        let local_keys: Vec<String> = state.files.keys().cloned().collect();
        for key in local_keys {
            if !remote_set.contains(key.as_str()) {
                state.files.remove(&key);
                let cache_path = self.cache_dir.join(&key);
                let _ = std::fs::remove_file(&cache_path);
                removed += 1;
            }
        }

        state.last_full_sync_at = chrono::Utc::now().timestamp();
        let state_clone = state.clone();
        drop(state);

        self.persist_state(&state_clone)?;

        if added > 0 || updated > 0 || removed > 0 {
            info!(
                "Obsidian sync: +{} added, ~{} updated, -{} removed",
                added, updated, removed
            );
        } else {
            debug!("Obsidian sync: no changes");
        }

        Ok(())
    }

    // --- Private helpers ---

    fn write_to_cache(&self, path: &str, content: &str) -> Result<()> {
        let file_path = self.cache_dir.join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::utils::atomic_write(&file_path, content)
    }

    async fn update_state_entry(&self, path: &str, content: &str) {
        let mut state = self.state.lock().await;
        let now = chrono::Utc::now().timestamp();
        state.files.insert(
            path.to_string(),
            CachedFileMeta {
                content_hash: content_hash(content),
                last_synced_at: now,
                size: content.len() as u64,
            },
        );
        let state_clone = state.clone();
        drop(state);
        if let Err(e) = self.persist_state(&state_clone) {
            warn!("Failed to persist sync state: {}", e);
        }
    }

    async fn enqueue_write(&self, path: &str, content: &str, operation: &str) -> Result<()> {
        let pre_hash = {
            let state = self.state.lock().await;
            state.files.get(path).map(|m| m.content_hash.clone())
        };

        let mut queue = self.write_queue.lock().await;
        queue.push(QueuedWrite {
            path: path.to_string(),
            content: content.to_string(),
            operation: operation.to_string(),
            queued_at: chrono::Utc::now().timestamp(),
            pre_write_hash: pre_hash,
        });
        drop(queue);
        self.persist_queue().await
    }

    fn persist_state(&self, state: &SyncState) -> Result<()> {
        let json = serde_json::to_string_pretty(state)?;
        crate::utils::atomic_write(&self.state_path, &json)
    }

    async fn persist_queue(&self) -> Result<()> {
        let queue = self.write_queue.lock().await;
        let json = serde_json::to_string_pretty(&*queue)?;
        drop(queue);
        crate::utils::atomic_write(&self.queue_path, &json)
    }
}

/// Background sync service that periodically syncs the Obsidian cache.
pub struct ObsidianSyncService {
    cache: Arc<ObsidianCache>,
    sync_interval: u64,
}

impl ObsidianSyncService {
    pub fn new(cache: Arc<ObsidianCache>, sync_interval: u64) -> Self {
        Self {
            cache,
            sync_interval,
        }
    }

    pub async fn start(&self) -> Result<()> {
        let cache = self.cache.clone();
        let interval = self.sync_interval;

        // Initial sync
        info!("Obsidian sync: running initial sync...");
        if cache.client.is_reachable().await {
            if let Err(e) = cache.flush_write_queue().await {
                warn!("Obsidian initial queue flush failed: {}", e);
            }
            if let Err(e) = cache.full_sync().await {
                warn!("Obsidian initial sync failed: {}", e);
            }
        } else {
            warn!("Obsidian API unreachable during initial sync — will retry");
        }

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(interval)).await;

                if cache.client.is_reachable().await {
                    if let Err(e) = cache.flush_write_queue().await {
                        warn!("Obsidian queue flush failed: {}", e);
                    }
                    if let Err(e) = cache.full_sync().await {
                        warn!("Obsidian sync failed: {}", e);
                    }
                } else {
                    debug!("Obsidian API unreachable, skipping sync tick");
                }
            }
        });

        Ok(())
    }
}
