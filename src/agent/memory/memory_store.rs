use crate::agent::memory::embeddings::EmbeddingService;
use crate::agent::memory::{MemoryDB, MemoryIndexer};
use crate::config::MemoryConfig;
use anyhow::{Context, Result};
use chrono::{Datelike, Utc};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, warn};

pub struct MemoryStore {
    memory_dir: PathBuf,
    db: Arc<MemoryDB>,
    indexer: Option<Arc<MemoryIndexer>>,
    embedding_service: Option<Arc<EmbeddingService>>,
    hybrid_weight: f32,
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

        // Create embedding service if enabled
        let embedding_service = if memory_config.embeddings_enabled {
            match EmbeddingService::new(&memory_config.embeddings_model) {
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
        })
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
            let today_key = format!(
                "{}-{:02}-{:02}.md",
                today.year(),
                today.month(),
                today.day()
            );
            let today_file = self.memory_dir.join(&today_key);
            if today_file.exists() {
                self.db.index_file(&today_key, &today_file)?;
            }
        }

        // Get today's date for daily notes
        let today = Utc::now();
        let today_key = format!(
            "{}-{:02}-{:02}.md",
            today.year(),
            today.month(),
            today.day()
        );
        let today_file = self.memory_dir.join(&today_key);

        // Search for relevant chunks if query provided
        let mut chunks = Vec::new();
        if let Some(query) = query {
            let exclude: HashSet<String> = [today_key.clone()].iter().cloned().collect();
            let hits = self.db.search(query, 8, Some(&exclude))?;
            for hit in hits {
                chunks.push(format!("**{}**: {}", hit.source_key, hit.content));
            }
        }

        debug!("memory context: {} chunks from query", chunks.len());

        // Always include MEMORY.md content (fallback when no query or no FTS results)
        if (chunks.is_empty() || query.is_none())
            && let Ok(long_term) = self.read_long_term()
            && !long_term.trim().is_empty()
        {
            chunks.insert(0, format!("## Long-term Memory\n{}", long_term));
        }

        // Include today's note
        if today_file.exists()
            && let Ok(content) = std::fs::read_to_string(&today_file)
            && !content.trim().is_empty()
        {
            chunks.push(format!("**Today's Notes ({})**:\n{}", today_key, content));
        }

        Ok(chunks.join("\n\n---\n\n"))
    }

    pub fn get_today_file(&self) -> PathBuf {
        let today = Utc::now();
        self.memory_dir.join(format!(
            "{}-{:02}-{:02}.md",
            today.year(),
            today.month(),
            today.day()
        ))
    }

    pub fn append_today(&self, content: &str) -> Result<()> {
        let today_file = self.get_today_file();
        let today = Utc::now();
        let date_str = format!("{}-{:02}-{:02}", today.year(), today.month(), today.day());

        if today_file.exists() {
            let existing = std::fs::read_to_string(&today_file)?;
            let new_content = format!("{}\n{}", existing, content);
            std::fs::write(&today_file, new_content)?;
        } else {
            let header = format!("# {}\n\n", date_str);
            std::fs::write(&today_file, format!("{}{}", header, content))?;
        }
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
}
