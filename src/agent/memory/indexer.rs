use crate::agent::memory::MemoryDB;
/// Background memory indexer service
///
/// Periodically indexes memory files in the background to avoid blocking queries.
use crate::agent::memory::embeddings::EmbeddingService;
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
    archive_after_days: u32,
    purge_after_days: u32,
    embedding_service: Option<Arc<EmbeddingService>>,
}

impl MemoryIndexer {
    pub fn new(db: Arc<MemoryDB>, memory_dir: PathBuf, interval_secs: u64) -> Self {
        Self {
            db,
            memory_dir,
            interval: Duration::from_secs(interval_secs),
            running: Arc::new(tokio::sync::Mutex::new(false)),
            last_index_time: Arc::new(Mutex::new(None)),
            archive_after_days: 30,
            purge_after_days: 90,
            embedding_service: None,
        }
    }

    pub fn with_hygiene_config(
        db: Arc<MemoryDB>,
        memory_dir: PathBuf,
        interval_secs: u64,
        archive_after_days: u32,
        purge_after_days: u32,
    ) -> Self {
        Self {
            db,
            memory_dir,
            interval: Duration::from_secs(interval_secs),
            running: Arc::new(tokio::sync::Mutex::new(false)),
            last_index_time: Arc::new(Mutex::new(None)),
            archive_after_days,
            purge_after_days,
            embedding_service: None,
        }
    }

    pub fn with_full_config(
        db: Arc<MemoryDB>,
        memory_dir: PathBuf,
        interval_secs: u64,
        archive_after_days: u32,
        purge_after_days: u32,
        embedding_service: Option<Arc<EmbeddingService>>,
    ) -> Self {
        Self {
            db,
            memory_dir,
            interval: Duration::from_secs(interval_secs),
            running: Arc::new(tokio::sync::Mutex::new(false)),
            last_index_time: Arc::new(Mutex::new(None)),
            archive_after_days,
            purge_after_days,
            embedding_service,
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
        let archive_days = self.archive_after_days;
        let purge_days = self.purge_after_days;
        let embedding_service = self.embedding_service.clone();

        // Do initial indexing immediately
        Self::index_memory_files(&db, &memory_dir, embedding_service.as_ref()).await;

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
                Self::index_memory_files(&db, &memory_dir, embedding_service.as_ref()).await;

                // Run hygiene after indexing
                Self::run_hygiene(&db, &memory_dir, archive_days, purge_days).await;

                last_index = std::time::Instant::now();
                *last_index_time.lock().await = Some(last_index);
            }

            info!("Memory indexer stopped");
        });

        info!("Memory indexer started (interval: {}s)", interval.as_secs());
        Ok(())
    }

    /// Stop the background indexing service
    pub async fn stop(&self) {
        let mut running = self.running.lock().await;
        *running = false;
    }

    /// Perform indexing (can be called manually)
    async fn index_memory_files(
        db: &MemoryDB,
        memory_dir: &Path,
        embedding_service: Option<&Arc<EmbeddingService>>,
    ) {
        debug!("Starting memory indexing...");
        match tokio::task::spawn_blocking({
            let db = db.clone();
            let memory_dir = memory_dir.to_path_buf();
            let embedding_service = embedding_service.cloned();
            move || {
                // Collect source keys that were indexed (for embedding generation)
                let mut indexed_sources: Vec<String> = Vec::new();

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
                    } else {
                        indexed_sources.push("MEMORY.md".to_string());
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
                    } else {
                        indexed_sources.push(today_key);
                    }
                }

                // Generate embeddings for indexed sources
                if let Some(ref emb_svc) = embedding_service {
                    Self::generate_embeddings_for_sources(&db, emb_svc, &indexed_sources);
                }

                debug!("Memory indexing completed");
            }
        })
        .await
        {
            Ok(()) => {}
            Err(e) => {
                warn!("Memory indexing task panicked: {}", e);
            }
        }
    }

    /// Generate embeddings for entries belonging to the given source keys.
    fn generate_embeddings_for_sources(
        db: &MemoryDB,
        embedding_service: &EmbeddingService,
        source_keys: &[String],
    ) {
        for source_key in source_keys {
            let entries = match db.get_entries_for_source(source_key) {
                Ok(e) => e,
                Err(e) => {
                    warn!("failed to get entries for {}: {}", source_key, e);
                    continue;
                }
            };

            if entries.is_empty() {
                continue;
            }

            let texts: Vec<&str> = entries
                .iter()
                .map(|(_, content)| content.as_str())
                .collect();
            match embedding_service.embed_texts(&texts) {
                Ok(embeddings) => {
                    for ((entry_id, _), emb) in entries.iter().zip(embeddings.iter()) {
                        let bytes = crate::agent::memory::embeddings::serialize_embedding(emb);
                        if let Err(e) = db.store_embedding(*entry_id, &bytes) {
                            warn!("failed to store embedding for entry {}: {}", entry_id, e);
                        }
                    }
                    debug!("generated {} embeddings for {}", entries.len(), source_key);
                }
                Err(e) => {
                    warn!("embedding generation failed for {}: {}", source_key, e);
                }
            }
        }
    }

    /// Run memory hygiene (archive/purge/cleanup) in a blocking task.
    async fn run_hygiene(db: &MemoryDB, memory_dir: &Path, archive_days: u32, purge_days: u32) {
        if archive_days == 0 && purge_days == 0 {
            return;
        }
        match tokio::task::spawn_blocking({
            let db = db.clone();
            let memory_dir = memory_dir.to_path_buf();
            move || {
                crate::agent::memory::hygiene::run_hygiene(
                    &db,
                    &memory_dir,
                    archive_days,
                    purge_days,
                );
            }
        })
        .await
        {
            Ok(()) => {}
            Err(e) => {
                warn!("Memory hygiene task panicked: {}", e);
            }
        }
    }

    /// Trigger immediate indexing (non-blocking)
    pub fn trigger_index(&self) {
        let db = self.db.clone();
        let memory_dir = self.memory_dir.clone();
        let embedding_service = self.embedding_service.clone();
        tokio::spawn(async move {
            Self::index_memory_files(&db, &memory_dir, embedding_service.as_ref()).await;
        });
    }
}
