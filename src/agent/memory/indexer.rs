/// Background memory indexer service
///
/// Periodically indexes memory files in the background to avoid blocking queries.
use crate::agent::memory::MemoryDB;
use anyhow::Result;
use chrono::Datelike;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

pub struct MemoryIndexer {
    db: Arc<MemoryDB>,
    memory_dir: PathBuf,
    interval: Duration,
    running: Arc<tokio::sync::Mutex<bool>>,
    last_index_time: Arc<Mutex<Option<std::time::Instant>>>,
}

impl MemoryIndexer {
    pub fn new(db: Arc<MemoryDB>, memory_dir: PathBuf, interval_secs: u64) -> Self {
        Self {
            db,
            memory_dir,
            interval: Duration::from_secs(interval_secs),
            running: Arc::new(tokio::sync::Mutex::new(false)),
            last_index_time: Arc::new(Mutex::new(None)),
        }
    }

    /// Start the background indexing service
    pub async fn start(&self) -> Result<()> {
        let mut running = self.running.lock().await;
        if *running {
            return Ok(()); // Already running
        }
        *running = true;
        drop(running);

        let db = self.db.clone();
        let memory_dir = self.memory_dir.clone();
        let running_clone = self.running.clone();
        let last_index_time = self.last_index_time.clone();
        let interval = self.interval;

        // Do initial indexing immediately
        Self::index_memory_files(&db, &memory_dir).await;

        tokio::spawn(async move {
            let mut last_index = std::time::Instant::now();
            *last_index_time.lock().await = Some(last_index);

            loop {
                // Check if we should stop
                {
                    let running_guard = running_clone.lock().await;
                    if !*running_guard {
                        break;
                    }
                }

                // Wait for interval
                tokio::time::sleep(interval).await;

                // Check again after sleep
                {
                    let running_guard = running_clone.lock().await;
                    if !*running_guard {
                        break;
                    }
                }

                // Perform indexing
                Self::index_memory_files(&db, &memory_dir).await;
                last_index = std::time::Instant::now();
                *last_index_time.lock().await = Some(last_index);
            }

            info!("Memory indexer stopped");
        });

        info!("Memory indexer started (interval: {}s)", interval.as_secs());
        Ok(())
    }

    /// Stop the background indexing service
    #[allow(dead_code)] // May be used for graceful shutdown in future
    pub async fn stop(&self) {
        let mut running = self.running.lock().await;
        *running = false;
    }

    /// Check if indexer is running
    #[allow(dead_code)] // May be used for monitoring in future
    pub async fn is_running(&self) -> bool {
        let running = self.running.lock().await;
        *running
    }

    /// Get the last index time
    #[allow(dead_code)] // May be used for monitoring in future
    pub async fn last_index_time(&self) -> Option<std::time::Instant> {
        let last_index = self.last_index_time.lock().await;
        *last_index
    }

    /// Perform indexing (can be called manually)
    async fn index_memory_files(db: &MemoryDB, memory_dir: &Path) {
        debug!("Starting memory indexing...");
        match tokio::task::spawn_blocking({
            let db = db.clone();
            let memory_dir = memory_dir.to_path_buf();
            move || {
                // Index directory
                if let Err(e) = db.index_directory(&memory_dir) {
                    warn!("Failed to index memory directory: {}", e);
                    return;
                }

                // Index MEMORY.md
                let memory_file = memory_dir.join("MEMORY.md");
                if memory_file.exists() {
                    if let Err(e) = db.index_file("MEMORY.md", &memory_file) {
                        warn!("Failed to index MEMORY.md: {}", e);
                    }
                }

                // Index today's note
                let today = chrono::Utc::now().date_naive();
                let today_key = format!(
                    "{}-{:02}-{:02}.md",
                    today.year(),
                    today.month(),
                    today.day()
                );
                let today_file = memory_dir.join(&today_key);
                if today_file.exists() {
                    if let Err(e) = db.index_file(&today_key, &today_file) {
                        warn!("Failed to index today's note: {}", e);
                    }
                }

                debug!("Memory indexing completed");
            }
        })
        .await
        {
            Ok(_) => {}
            Err(e) => {
                warn!("Memory indexing task panicked: {}", e);
            }
        }
    }

    /// Trigger immediate indexing (non-blocking)
    pub fn trigger_index(&self) {
        let db = self.db.clone();
        let memory_dir = self.memory_dir.clone();
        tokio::spawn(async move {
            Self::index_memory_files(&db, &memory_dir).await;
        });
    }
}
