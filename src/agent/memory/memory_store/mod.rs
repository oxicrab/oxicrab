use crate::agent::memory::MemoryDB;
#[cfg(feature = "embeddings")]
use crate::agent::memory::embeddings::{EmbeddingService, LazyEmbeddingService};
use crate::config::MemoryConfig;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tracing::debug;
#[cfg(feature = "embeddings")]
use tracing::warn;

pub struct MemoryStore {
    db: Arc<MemoryDB>,
    #[cfg(feature = "embeddings")]
    embedding_service: Option<Arc<LazyEmbeddingService>>,
    #[cfg(feature = "embeddings")]
    hybrid_weight: f32,
    #[cfg(feature = "embeddings")]
    fusion_strategy: crate::config::FusionStrategy,
    #[cfg(feature = "embeddings")]
    rrf_k: u32,
    #[cfg(feature = "embeddings")]
    recency_half_life_days: u32,
}

impl MemoryStore {
    pub fn new(workspace: impl AsRef<Path>) -> Result<Self> {
        let workspace = workspace.as_ref();
        let memory_dir = workspace.join("memory");

        // Ensure workspace and memory dir exist (DB still lives in memory/)
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

        Ok(Self {
            db,
            #[cfg(feature = "embeddings")]
            embedding_service: None,
            #[cfg(feature = "embeddings")]
            hybrid_weight: 0.5,
            #[cfg(feature = "embeddings")]
            fusion_strategy: crate::config::FusionStrategy::default(),
            #[cfg(feature = "embeddings")]
            rrf_k: 60,
            #[cfg(feature = "embeddings")]
            recency_half_life_days: 90,
        })
    }

    pub fn with_config(
        workspace: impl AsRef<Path>,
        memory_config: &MemoryConfig,
        _workspace_ttl: &std::collections::HashMap<String, Option<u64>>,
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

        // Create embedding service if enabled (lazy background initialization)
        #[cfg(feature = "embeddings")]
        let embedding_service = if memory_config.embeddings_enabled {
            Some(Arc::new(LazyEmbeddingService::new(
                memory_config.embeddings_model.clone(),
                memory_config.embedding_cache_size,
            )))
        } else {
            None
        };

        Ok(Self {
            db,
            #[cfg(feature = "embeddings")]
            embedding_service,
            #[cfg(feature = "embeddings")]
            hybrid_weight: memory_config.hybrid_weight,
            #[cfg(feature = "embeddings")]
            fusion_strategy: memory_config.fusion_strategy,
            #[cfg(feature = "embeddings")]
            rrf_k: memory_config.rrf_k,
            #[cfg(feature = "embeddings")]
            recency_half_life_days: memory_config.recency_half_life_days,
        })
    }

    /// Accessor for the inner database (used for token logging and stats).
    pub fn db(&self) -> Arc<MemoryDB> {
        self.db.clone()
    }

    /// Get the embedding service if ready (None if disabled or still initializing).
    #[cfg(feature = "embeddings")]
    pub fn embedding_service(&self) -> Option<&EmbeddingService> {
        self.embedding_service.as_ref().and_then(|lazy| lazy.get())
    }

    /// Whether embeddings are available for hybrid search.
    pub fn has_embeddings(&self) -> bool {
        #[cfg(feature = "embeddings")]
        {
            self.embedding_service
                .as_ref()
                .is_some_and(|s| s.is_ready())
        }
        #[cfg(not(feature = "embeddings"))]
        false
    }

    /// Hybrid search combining keyword and vector similarity.
    #[cfg(feature = "embeddings")]
    pub fn hybrid_search(
        &self,
        query: &str,
        limit: usize,
        exclude_sources: Option<&HashSet<String>>,
    ) -> Result<Vec<crate::agent::memory::memory_db::MemoryHit>> {
        let emb_svc = self
            .embedding_service
            .as_ref()
            .and_then(|lazy| lazy.get())
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
            self.recency_half_life_days,
        )?;
        debug!(
            "memory hybrid search: query_len={}, results={}",
            query.len(),
            hits.len()
        );
        Ok(hits)
    }

    pub fn get_memory_context(&self, query: Option<&str>) -> Result<String> {
        self.get_memory_context_scoped(query, false)
    }

    /// Get memory context with optional group scoping.
    /// When `is_group` is true, personal daily notes are excluded from search results.
    pub fn get_memory_context_scoped(&self, query: Option<&str>, is_group: bool) -> Result<String> {
        let mut chunks = Vec::new();
        if let Some(query) = query {
            let exclude: HashSet<String> = HashSet::new();
            let fetch_limit = if is_group { 16 } else { 8 };
            let result_limit = 8;
            let hits = if self.has_embeddings() {
                #[cfg(feature = "embeddings")]
                {
                    match self.hybrid_search(query, fetch_limit, Some(&exclude)) {
                        Ok(h) => h,
                        Err(e) => {
                            warn!("hybrid search failed, falling back to keyword: {}", e);
                            self.db.search(query, fetch_limit, Some(&exclude))?
                        }
                    }
                }
                #[cfg(not(feature = "embeddings"))]
                {
                    self.db.search(query, fetch_limit, Some(&exclude))?
                }
            } else {
                self.db.search(query, fetch_limit, Some(&exclude))?
            };
            for hit in hits {
                // In group mode, skip hits from daily notes (daily:YYYY-MM-DD prefix)
                if is_group && hit.source_key.starts_with("daily:") {
                    continue;
                }
                chunks.push(format!("**{}**: {}", hit.source_key, hit.content));
                if chunks.len() >= result_limit {
                    break;
                }
            }
        }

        debug!(
            "memory context: {} chunks from query (is_group={})",
            chunks.len(),
            is_group
        );

        Ok(chunks.join("\n\n---\n\n"))
    }

    /// Append content to today's daily notes in the DB.
    pub fn append_today(&self, content: &str) -> Result<()> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let source_key = format!("daily:{today}");
        self.db.insert_memory(&source_key, content.trim())
    }

    /// Append content under a named section for today's notes in the DB.
    /// The section name is embedded in the source key for organization.
    pub fn append_to_section(&self, section: &str, content: &str) -> Result<()> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let source_key = format!("daily:{today}:{section}");
        self.db.insert_memory(&source_key, content.trim())
    }

    /// Get recent daily entries for deduplication.
    pub fn get_recent_daily_entries(&self, limit: usize) -> Result<Vec<String>> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let source_key = format!("daily:{today}");
        self.db.get_recent_entries(&source_key, limit)
    }

    /// Read entries from a specific section of today's notes.
    pub fn read_today_section(&self, section: &str) -> Result<String> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let source_key = format!("daily:{today}:{section}");
        let entries = self.db.get_recent_entries(&source_key, 100)?;
        Ok(entries.join("\n"))
    }
}

#[cfg(test)]
mod tests;
