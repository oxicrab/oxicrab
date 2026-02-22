use crate::agent::memory::embeddings::EmbeddingService;
use crate::agent::memory::{MemoryDB, MemoryIndexer};
use crate::config::MemoryConfig;
use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, warn};

/// Check if a source key is a daily note file (e.g. "2026-02-22.md").
fn is_daily_note_key(key: &str) -> bool {
    key.len() == 13
        && Path::new(key)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        && key.as_bytes()[4] == b'-'
        && key.as_bytes()[7] == b'-'
        && key[..4].bytes().all(|b| b.is_ascii_digit())
}

pub struct MemoryStore {
    memory_dir: PathBuf,
    db: Arc<MemoryDB>,
    indexer: Option<Arc<MemoryIndexer>>,
    embedding_service: Option<Arc<EmbeddingService>>,
    hybrid_weight: f32,
    fusion_strategy: crate::config::FusionStrategy,
    rrf_k: u32,
}

impl MemoryStore {
    pub fn new(workspace: impl AsRef<Path>) -> Result<Self> {
        Self::with_indexer_interval(workspace, 300)
    }

    pub fn with_config(
        workspace: impl AsRef<Path>,
        indexer_interval_secs: u64,
        memory_config: &MemoryConfig,
    ) -> Result<Self> {
        let workspace = workspace.as_ref();
        let memory_dir = workspace.join("memory");

        std::fs::create_dir_all(workspace).with_context(|| {
            format!(
                "Failed to create workspace directory: {}",
                workspace.display()
            )
        })?;

        std::fs::create_dir_all(&memory_dir).with_context(|| {
            format!(
                "Failed to create memory directory: {}",
                memory_dir.display()
            )
        })?;

        let db_path = memory_dir.join("memory.sqlite3");
        let db_path_clone = db_path.clone();
        let db = Arc::new(MemoryDB::new(db_path).with_context(|| {
            format!(
                "Failed to create memory database at: {}",
                db_path_clone.display()
            )
        })?);

        // Create embedding service if enabled (with configurable cache size)
        let embedding_service = if memory_config.embeddings_enabled {
            match EmbeddingService::with_cache_size(
                &memory_config.embeddings_model,
                memory_config.embedding_cache_size,
            ) {
                Ok(svc) => Some(Arc::new(svc)),
                Err(e) => {
                    warn!("failed to initialize embedding service: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let indexer = Arc::new(MemoryIndexer::with_full_config(
            db.clone(),
            memory_dir.clone(),
            indexer_interval_secs,
            memory_config.archive_after_days,
            memory_config.purge_after_days,
            embedding_service.clone(),
        ));

        Ok(Self {
            memory_dir,
            db,
            indexer: Some(indexer),
            embedding_service,
            hybrid_weight: memory_config.hybrid_weight,
            fusion_strategy: memory_config.fusion_strategy,
            rrf_k: memory_config.rrf_k,
        })
    }

    pub fn with_indexer_interval(
        workspace: impl AsRef<Path>,
        indexer_interval_secs: u64,
    ) -> Result<Self> {
        let workspace = workspace.as_ref();
        let memory_dir = workspace.join("memory");

        // Ensure workspace exists first
        std::fs::create_dir_all(workspace).with_context(|| {
            format!(
                "Failed to create workspace directory: {}",
                workspace.display()
            )
        })?;

        std::fs::create_dir_all(&memory_dir).with_context(|| {
            format!(
                "Failed to create memory directory: {}",
                memory_dir.display()
            )
        })?;

        let db_path = memory_dir.join("memory.sqlite3");
        let db_path_clone = db_path.clone();
        let db = Arc::new(MemoryDB::new(db_path).with_context(|| {
            format!(
                "Failed to create memory database at: {}",
                db_path_clone.display()
            )
        })?);

        // Create background indexer
        // Note: Indexer will be started separately via start_indexer() to allow sync initialization
        let indexer = Arc::new(MemoryIndexer::new(
            db.clone(),
            memory_dir.clone(),
            indexer_interval_secs,
        ));

        Ok(Self {
            memory_dir,
            db,
            indexer: Some(indexer),
            embedding_service: None,
            hybrid_weight: 0.5,
            fusion_strategy: crate::config::FusionStrategy::default(),
            rrf_k: 60,
        })
    }

    /// Accessor for the inner database (used by `CostGuard` for persistence).
    pub fn db(&self) -> Arc<MemoryDB> {
        self.db.clone()
    }

    /// Whether embeddings are available for hybrid search.
    pub fn has_embeddings(&self) -> bool {
        self.embedding_service.is_some()
    }

    /// Hybrid search combining keyword and vector similarity.
    pub fn hybrid_search(
        &self,
        query: &str,
        limit: usize,
        exclude_sources: Option<&HashSet<String>>,
    ) -> Result<Vec<crate::agent::memory::memory_db::MemoryHit>> {
        let emb_svc = self
            .embedding_service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("embeddings not available"))?;
        let query_embedding = emb_svc.embed_query(query)?;
        let hits = self.db.hybrid_search(
            query,
            &query_embedding,
            limit,
            exclude_sources,
            self.hybrid_weight,
            self.fusion_strategy,
            self.rrf_k,
        )?;
        debug!(
            "memory hybrid search: query_len={}, results={}",
            query.len(),
            hits.len()
        );
        Ok(hits)
    }

    /// Start the background memory indexer
    /// This should be called after construction if background indexing is desired
    pub async fn start_indexer(&self) -> Result<()> {
        if let Some(ref indexer) = self.indexer {
            indexer.start().await?;
        }
        Ok(())
    }

    /// Stop the background memory indexer
    pub async fn stop_indexer(&self) {
        if let Some(ref indexer) = self.indexer {
            indexer.stop().await;
        }
    }

    pub fn get_memory_context(&self, query: Option<&str>) -> Result<String> {
        self.get_memory_context_scoped(query, false)
    }

    /// Get memory context with optional group scoping.
    /// When `is_group` is true, personal memory (MEMORY.md, daily notes) is excluded.
    pub fn get_memory_context_scoped(&self, query: Option<&str>, is_group: bool) -> Result<String> {
        // Trigger background indexing if indexer is available
        // This ensures fresh indexing without blocking the query
        if let Some(ref indexer) = self.indexer {
            indexer.trigger_index();
        } else {
            // Fallback: index synchronously if indexer not available
            // This should rarely happen, but provides backward compatibility
            self.db.index_directory(&self.memory_dir)?;
            let memory_file = self.memory_dir.join("MEMORY.md");
            if memory_file.exists() {
                self.db.index_file("MEMORY.md", &memory_file)?;
            }
            let today = Utc::now();
            let today_key = format!("{}.md", today.format("%Y-%m-%d"));
            let today_file = self.memory_dir.join(&today_key);
            if today_file.exists() {
                self.db.index_file(&today_key, &today_file)?;
            }
        }

        // Get today's date for daily notes
        let today = Utc::now();
        let today_key = format!("{}.md", today.format("%Y-%m-%d"));
        let today_file = self.memory_dir.join(&today_key);

        // Search for relevant chunks if query provided
        let mut chunks = Vec::new();
        if let Some(query) = query {
            let mut exclude: HashSet<String> = [today_key.clone()].into_iter().collect();
            // In group chats, also exclude personal memory files from search results
            if is_group {
                exclude.insert("MEMORY.md".to_string());
            }
            let hits = if self.has_embeddings() {
                match self.hybrid_search(query, 8, Some(&exclude)) {
                    Ok(h) => h,
                    Err(e) => {
                        warn!("hybrid search failed, falling back to keyword: {}", e);
                        self.db.search(query, 8, Some(&exclude))?
                    }
                }
            } else {
                self.db.search(query, 8, Some(&exclude))?
            };
            for hit in hits {
                // In group mode, skip hits from daily notes (YYYY-MM-DD.md pattern)
                if is_group && is_daily_note_key(&hit.source_key) {
                    continue;
                }
                chunks.push(format!("**{}**: {}", hit.source_key, hit.content));
            }
        }

        debug!(
            "memory context: {} chunks from query (is_group={})",
            chunks.len(),
            is_group
        );

        // Include MEMORY.md content only in DM/private chats
        if !is_group
            && (chunks.is_empty() || query.is_none())
            && let Ok(long_term) = self.read_long_term()
            && !long_term.trim().is_empty()
        {
            chunks.insert(0, format!("## Long-term Memory\n{}", long_term));
        }

        // Include today's note only in DM/private chats
        if !is_group && today_file.exists() {
            let _lock = Self::lock_daily_shared(&today_file);
            if let Ok(content) = std::fs::read_to_string(&today_file)
                && !content.trim().is_empty()
            {
                chunks.push(format!("**Today's Notes ({})**:\n{}", today_key, content));
            }
        }

        Ok(chunks.join("\n\n---\n\n"))
    }

    pub fn get_today_file(&self) -> PathBuf {
        let today = Utc::now();
        self.memory_dir
            .join(format!("{}.md", today.format("%Y-%m-%d")))
    }

    pub fn append_today(&self, content: &str) -> Result<()> {
        use fs2::FileExt;
        use std::io::Write;
        let today_file = self.get_today_file();
        let today = Utc::now();
        let date_str = today.format("%Y-%m-%d").to_string();

        // Cross-process lock to prevent CLI + daemon from corrupting daily notes
        let lock_path = today_file.with_extension("md.lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .with_context(|| "failed to open memory notes lock file")?;
        lock_file
            .lock_exclusive()
            .with_context(|| "failed to acquire memory notes lock")?;

        if !today_file.exists() {
            let header = format!("# {}\n\n", date_str);
            std::fs::write(&today_file, header)?;
        }
        let mut file = std::fs::OpenOptions::new().append(true).open(&today_file)?;
        writeln!(file, "{}", content)?;
        // lock released when lock_file drops
        Ok(())
    }

    pub fn read_long_term(&self) -> Result<String> {
        let memory_file = self.memory_dir.join("MEMORY.md");
        if memory_file.exists() {
            Ok(std::fs::read_to_string(&memory_file)?)
        } else {
            Ok(String::new())
        }
    }

    /// Acquire a shared lock on the daily notes lock file.
    /// Returns None if the lock file cannot be created (non-fatal).
    fn lock_daily_shared(today_file: &Path) -> Option<std::fs::File> {
        let lock_path = today_file.with_extension("md.lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .ok()?;
        fs2::FileExt::lock_shared(&lock_file).ok()?;
        Some(lock_file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_daily_note_key() {
        assert!(is_daily_note_key("2026-02-22.md"));
        assert!(is_daily_note_key("2025-12-31.md"));
        assert!(!is_daily_note_key("MEMORY.md"));
        assert!(!is_daily_note_key("notes.md"));
        assert!(!is_daily_note_key("2026-02-22.txt"));
        assert!(!is_daily_note_key("2026-02-22"));
        assert!(!is_daily_note_key(""));
    }

    #[test]
    fn test_with_config_wires_fusion_strategy() {
        let tmp = tempfile::TempDir::new().unwrap();
        let memory_dir = tmp.path().join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();

        let mut config = crate::config::MemoryConfig::default();
        config.fusion_strategy = crate::config::FusionStrategy::Rrf;
        config.rrf_k = 42;
        config.hybrid_weight = 0.3;

        let store = MemoryStore::with_config(tmp.path(), 0, &config).unwrap();
        assert_eq!(store.fusion_strategy, crate::config::FusionStrategy::Rrf);
        assert_eq!(store.rrf_k, 42);
        assert!((store.hybrid_weight - 0.3).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn test_group_memory_context_excludes_personal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let memory_dir = tmp.path().join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(memory_dir.join("MEMORY.md"), "personal secret data").unwrap();

        let store = MemoryStore::with_indexer_interval(tmp.path(), 0).unwrap();

        // Normal mode includes MEMORY.md
        let normal = store.get_memory_context(None).unwrap();
        assert!(
            normal.contains("personal secret data"),
            "DM context should include MEMORY.md"
        );

        // Group mode excludes MEMORY.md
        let group = store.get_memory_context_scoped(None, true).unwrap();
        assert!(
            !group.contains("personal secret data"),
            "group context should NOT include MEMORY.md"
        );
    }
}
