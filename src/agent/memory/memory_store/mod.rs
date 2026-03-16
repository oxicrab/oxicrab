use crate::agent::memory::MemoryDB;
#[cfg(feature = "embeddings")]
use crate::agent::memory::embeddings::{EmbeddingService, LazyEmbeddingService};
use crate::config::MemoryConfig;
use anyhow::{Context, Result};
use rusqlite::params;
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

    /// Create a `MemoryStore` using an existing `MemoryDB` instance.
    /// Avoids opening a second `SQLite` connection to the same file.
    pub fn with_db(db: Arc<MemoryDB>) -> Self {
        Self {
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
        }
    }

    /// Create a `MemoryStore` using an existing `MemoryDB` instance with config.
    pub fn with_db_and_config(db: Arc<MemoryDB>, memory_config: &MemoryConfig) -> Self {
        #[cfg(not(feature = "embeddings"))]
        let _ = memory_config;
        #[cfg(feature = "embeddings")]
        let embedding_service = if memory_config.embeddings_enabled {
            Some(Arc::new(LazyEmbeddingService::new(
                memory_config.embeddings_model.clone(),
                memory_config.embedding_cache_size,
            )))
        } else {
            None
        };

        Self {
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
        }
    }

    pub fn with_config(workspace: impl AsRef<Path>, memory_config: &MemoryConfig) -> Result<Self> {
        #[cfg(not(feature = "embeddings"))]
        let _ = memory_config;
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
            let exclude = if is_group {
                // In group mode, exclude daily: prefixed keys at query time
                // to avoid fetching results we'd discard.
                let daily_keys: HashSet<String> = self
                    .db
                    .list_daily_source_keys()
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
                daily_keys
            } else {
                HashSet::new()
            };
            let result_limit = 8;
            let hits = if self.has_embeddings() {
                #[cfg(feature = "embeddings")]
                {
                    match self.hybrid_search(query, result_limit, Some(&exclude)) {
                        Ok(h) => h,
                        Err(e) => {
                            warn!("hybrid search failed, falling back to keyword: {}", e);
                            self.db.search(query, result_limit, Some(&exclude))?
                        }
                    }
                }
                #[cfg(not(feature = "embeddings"))]
                {
                    self.db.search(query, result_limit, Some(&exclude))?
                }
            } else {
                self.db.search(query, result_limit, Some(&exclude))?
            };
            for hit in hits {
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
        self.db.insert_memory(&source_key, content.trim())?;
        self.backfill_embeddings();
        Ok(())
    }

    /// Append content under a named section for today's notes in the DB.
    /// The section name is embedded in the source key for organization.
    pub fn append_to_section(&self, section: &str, content: &str) -> Result<()> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let source_key = format!("daily:{today}:{section}");
        self.db.insert_memory(&source_key, content.trim())?;
        self.backfill_embeddings();
        Ok(())
    }

    /// Generate embeddings for any entries that don't have them yet.
    /// Best-effort: logs warnings on failure but never errors out.
    #[cfg_attr(not(feature = "embeddings"), allow(clippy::unused_self))]
    fn backfill_embeddings(&self) {
        #[cfg(feature = "embeddings")]
        if let Some(svc) = self.embedding_service() {
            match self.db.get_entries_missing_embeddings() {
                Ok(entries) if !entries.is_empty() => {
                    let texts: Vec<&str> = entries.iter().map(|(_, _, c)| c.as_str()).collect();
                    match svc.embed_texts(&texts) {
                        Ok(vectors) => {
                            for ((id, _, _), vec) in entries.iter().zip(vectors.iter()) {
                                let bytes =
                                    crate::agent::memory::embeddings::serialize_embedding(vec);
                                if let Err(e) = self.db.store_embedding(*id, &bytes) {
                                    warn!("failed to store embedding for entry {id}: {e}");
                                }
                            }
                            debug!("back-filled {} embeddings", entries.len());
                        }
                        Err(e) => warn!("embedding back-fill failed: {e}"),
                    }
                }
                Err(e) => warn!("failed to check for missing embeddings: {e}"),
                _ => {}
            }
        }
    }

    /// Get recent daily entries across all days for deduplication.
    pub fn get_recent_daily_entries(&self, limit: usize) -> Result<Vec<String>> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT content FROM memory_entries WHERE source_key LIKE 'daily:%' ORDER BY created_at DESC LIMIT ?",
        )?;
        let rows: Result<Vec<_>, _> = stmt.query_map(params![limit], |row| row.get(0))?.collect();
        rows.map_err(|e| anyhow::anyhow!("failed to get recent daily entries: {e}"))
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
