use crate::agent::memory::MemoryDB;
/// Background memory indexer service
///
/// Periodically indexes memory files in the background to avoid blocking queries.
use crate::agent::memory::embeddings::{EmbeddingService, LazyEmbeddingService};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

pub struct MemoryIndexer {
    db: Arc<MemoryDB>,
    memory_dir: PathBuf,
    knowledge_dir: Option<PathBuf>,
    interval: Duration,
    running: Arc<tokio::sync::Mutex<bool>>,
    last_index_time: Arc<Mutex<Option<std::time::Instant>>>,
    archive_after_days: u32,
    purge_after_days: u32,
    embedding_service: Option<Arc<LazyEmbeddingService>>,
    indexing_in_progress: Arc<std::sync::atomic::AtomicBool>,
    workspace_ttl: std::collections::HashMap<String, Option<u64>>,
}

impl MemoryIndexer {
    pub fn new(db: Arc<MemoryDB>, memory_dir: PathBuf, interval_secs: u64) -> Self {
        Self {
            db,
            memory_dir,
            knowledge_dir: None,
            interval: Duration::from_secs(interval_secs),
            running: Arc::new(tokio::sync::Mutex::new(false)),
            last_index_time: Arc::new(Mutex::new(None)),
            archive_after_days: 30,
            purge_after_days: 90,
            embedding_service: None,
            indexing_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            workspace_ttl: std::collections::HashMap::new(),
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
            knowledge_dir: None,
            interval: Duration::from_secs(interval_secs),
            running: Arc::new(tokio::sync::Mutex::new(false)),
            last_index_time: Arc::new(Mutex::new(None)),
            archive_after_days,
            purge_after_days,
            embedding_service: None,
            indexing_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            workspace_ttl: std::collections::HashMap::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_full_config(
        db: Arc<MemoryDB>,
        memory_dir: PathBuf,
        knowledge_dir: Option<PathBuf>,
        interval_secs: u64,
        archive_after_days: u32,
        purge_after_days: u32,
        embedding_service: Option<Arc<LazyEmbeddingService>>,
        workspace_ttl: std::collections::HashMap<String, Option<u64>>,
    ) -> Self {
        Self {
            db,
            memory_dir,
            knowledge_dir,
            interval: Duration::from_secs(interval_secs),
            running: Arc::new(tokio::sync::Mutex::new(false)),
            last_index_time: Arc::new(Mutex::new(None)),
            archive_after_days,
            purge_after_days,
            embedding_service,
            indexing_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            workspace_ttl,
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
        let knowledge_dir = self.knowledge_dir.clone();
        let running_clone = self.running.clone();
        let last_index_time = self.last_index_time.clone();
        let interval = self.interval;
        let archive_days = self.archive_after_days;
        let purge_days = self.purge_after_days;
        let embedding_service = self.embedding_service.clone();
        let workspace_ttl = self.workspace_ttl.clone();

        // Initial indexing now runs inside the spawn (non-blocking)
        tokio::spawn(async move {
            // Initial index
            Self::index_memory_files(
                &db,
                &memory_dir,
                knowledge_dir.as_deref(),
                embedding_service.as_ref(),
            )
            .await;

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
                Self::index_memory_files(
                    &db,
                    &memory_dir,
                    knowledge_dir.as_deref(),
                    embedding_service.as_ref(),
                )
                .await;

                // Run hygiene after indexing
                Self::run_hygiene(&db, &memory_dir, archive_days, purge_days, &workspace_ttl).await;

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
        knowledge_dir: Option<&Path>,
        embedding_service: Option<&Arc<LazyEmbeddingService>>,
    ) {
        debug!("Starting memory indexing...");
        match tokio::task::spawn_blocking({
            let db = db.clone();
            let memory_dir = memory_dir.to_path_buf();
            let knowledge_dir = knowledge_dir.map(Path::to_path_buf);
            let embedding_service = embedding_service.cloned();
            move || {
                // Collect source keys that were indexed (for embedding generation)
                let mut indexed_sources: Vec<String> = Vec::new();

                // Index directory (continue on failure — MEMORY.md and today still need indexing)
                if let Err(e) = db.index_directory(&memory_dir) {
                    warn!("failed to index memory directory: {}", e);
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
                let today_key = format!("{}.md", today.format("%Y-%m-%d"));
                let today_file = memory_dir.join(&today_key);
                if today_file.exists() {
                    if let Err(e) = db.index_file(&today_key, &today_file) {
                        warn!("Failed to index today's note: {}", e);
                    } else {
                        indexed_sources.push(today_key);
                    }
                }

                // Index knowledge directory (if configured)
                if let Some(ref kdir) = knowledge_dir
                    && kdir.is_dir()
                    && let Err(e) = db.index_knowledge_directory(kdir)
                {
                    warn!("failed to index knowledge directory: {}", e);
                }

                // Unwrap lazy service — if still loading, skip embeddings this cycle
                let inner_embedding_service =
                    embedding_service.as_ref().and_then(|lazy| lazy.get());

                // Generate embeddings for indexed sources
                if let Some(emb_svc) = inner_embedding_service {
                    Self::generate_embeddings_for_sources(&db, emb_svc, &indexed_sources);
                    Self::backfill_missing_embeddings(&db, emb_svc);
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

    /// Back-fill embeddings for entries that were indexed before embeddings were enabled.
    fn backfill_missing_embeddings(db: &MemoryDB, embedding_service: &EmbeddingService) {
        let missing = match db.get_entries_missing_embeddings() {
            Ok(m) => m,
            Err(e) => {
                warn!("failed to get entries missing embeddings: {}", e);
                return;
            }
        };

        if missing.is_empty() {
            return;
        }

        info!("back-filling embeddings for {} entries", missing.len());

        // Group by source_key for batch processing
        let mut by_source: std::collections::HashMap<String, Vec<(i64, String)>> =
            std::collections::HashMap::new();
        for (id, source_key, content) in missing {
            by_source.entry(source_key).or_default().push((id, content));
        }

        for (source_key, entries) in &by_source {
            let texts: Vec<&str> = entries
                .iter()
                .map(|(_, content)| content.as_str())
                .collect();
            match embedding_service.embed_texts(&texts) {
                Ok(embeddings) => {
                    for ((entry_id, _), emb) in entries.iter().zip(embeddings.iter()) {
                        let bytes = crate::agent::memory::embeddings::serialize_embedding(emb);
                        if let Err(e) = db.store_embedding(*entry_id, &bytes) {
                            warn!(
                                "failed to store back-fill embedding for entry {}: {}",
                                entry_id, e
                            );
                        }
                    }
                    debug!(
                        "back-filled {} embeddings for {}",
                        entries.len(),
                        source_key
                    );
                }
                Err(e) => {
                    warn!(
                        "back-fill embedding generation failed for {}: {}",
                        source_key, e
                    );
                }
            }
        }
    }

    /// Run memory hygiene (archive/purge/cleanup) in a blocking task.
    async fn run_hygiene(
        db: &MemoryDB,
        memory_dir: &Path,
        archive_days: u32,
        purge_days: u32,
        workspace_ttl: &std::collections::HashMap<String, Option<u64>>,
    ) {
        if archive_days == 0 && purge_days == 0 && workspace_ttl.is_empty() {
            return;
        }
        match tokio::task::spawn_blocking({
            let db = db.clone();
            let memory_dir = memory_dir.to_path_buf();
            let workspace_ttl = workspace_ttl.clone();
            move || {
                crate::agent::memory::hygiene::run_hygiene(
                    &db,
                    &memory_dir,
                    archive_days,
                    purge_days,
                );
                // Workspace root is the parent of the memory directory
                if !workspace_ttl.is_empty()
                    && let Some(workspace_root) = memory_dir.parent()
                    && let Err(e) = crate::agent::memory::hygiene::cleanup_workspace_files(
                        &db,
                        workspace_root,
                        &workspace_ttl,
                    )
                {
                    warn!("workspace file cleanup failed: {e}");
                }
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

    /// Trigger immediate indexing (non-blocking).
    /// Skips if an indexing task is already in progress.
    pub fn trigger_index(&self) {
        if self
            .indexing_in_progress
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_err()
        {
            debug!("indexing already in progress, skipping trigger");
            return;
        }
        let db = self.db.clone();
        let memory_dir = self.memory_dir.clone();
        let knowledge_dir = self.knowledge_dir.clone();
        let embedding_service = self.embedding_service.clone();
        let flag = self.indexing_in_progress.clone();
        tokio::spawn(async move {
            // Use a struct guard to ensure the flag is always cleared,
            // even if index_memory_files panics (prevents permanent deadlock)
            struct FlagGuard(Arc<std::sync::atomic::AtomicBool>);
            impl Drop for FlagGuard {
                fn drop(&mut self) {
                    self.0.store(false, std::sync::atomic::Ordering::Release);
                }
            }
            let _guard = FlagGuard(flag);
            Self::index_memory_files(
                &db,
                &memory_dir,
                knowledge_dir.as_deref(),
                embedding_service.as_ref(),
            )
            .await;
        });
    }
}
