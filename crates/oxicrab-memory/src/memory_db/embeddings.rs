use super::MemoryDB;
use anyhow::Result;
use rusqlite::params;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::warn;

/// Cached deserialized embedding for in-memory vector search.
#[derive(Clone)]
pub(super) struct CachedEmbedding {
    pub entry_id: i64,
    pub source_key: String,
    pub content: String,
    pub embedding: Vec<f32>,
}

impl MemoryDB {
    /// Store an embedding for a memory entry.
    ///
    /// Invalidates the in-memory embedding cache so the next `hybrid_search`
    /// picks up the new data.
    pub fn store_embedding(&self, entry_id: i64, embedding: &[u8]) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO memory_embeddings (entry_id, embedding) VALUES (?, ?)",
            params![entry_id, embedding],
        )?;
        self.invalidate_embedding_cache();
        Ok(())
    }

    /// Get all embeddings, optionally excluding certain source keys.
    /// Returns (`entry_id`, `source_key`, content, `embedding_blob`).
    #[allow(clippy::type_complexity)]
    pub(super) fn get_all_embeddings(
        &self,
        exclude_sources: Option<&std::collections::HashSet<String>>,
    ) -> Result<Vec<(i64, String, String, Vec<u8>)>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT me.id, me.source_key, me.content, emb.embedding
             FROM memory_embeddings emb
             JOIN memory_entries me ON emb.entry_id = me.id",
        )?;

        let default_set = std::collections::HashSet::new();
        let exclude = exclude_sources.unwrap_or(&default_set);

        let rows: Result<Vec<_>, _> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            })?
            .collect();

        Ok(rows
            .map_err(|e| anyhow::anyhow!("Failed to get embeddings: {e}"))?
            .into_iter()
            .filter(|(_, source_key, _, _)| !exclude.contains(source_key))
            .collect())
    }

    /// Get or populate the in-memory embedding cache.
    /// Returns cached deserialized embeddings, loading from DB on first call
    /// or after invalidation (e.g. after `store_embedding`).
    pub(super) fn get_cached_embeddings(
        &self,
        exclude_sources: Option<&std::collections::HashSet<String>>,
    ) -> Result<Vec<CachedEmbedding>> {
        use crate::embeddings::deserialize_embedding;

        let default_set = std::collections::HashSet::new();
        let exclude = exclude_sources.unwrap_or(&default_set);

        // Snapshot the current generation before checking the cache.
        // Acquire ordering pairs with the Release in store_embedding.
        let current_gen = self.embedding_generation.load(Ordering::Acquire);

        // Check cache first — only use it if its generation matches.
        {
            let cache = self
                .embedding_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((cached_gen, ref cached)) = *cache
                && cached_gen == current_gen
            {
                let shared = Arc::clone(cached);
                drop(cache);
                return Ok(shared
                    .iter()
                    .filter(|e| !exclude.contains(&e.source_key))
                    .cloned()
                    .collect());
            }
        }

        // Cache miss or stale — load from DB, deserialize, and cache
        let raw = self.get_all_embeddings(None)?;
        let mut entries = Vec::with_capacity(raw.len());
        for (entry_id, source_key, content, emb_bytes) in raw {
            match deserialize_embedding(&emb_bytes) {
                Ok(embedding) => entries.push(CachedEmbedding {
                    entry_id,
                    source_key,
                    content,
                    embedding,
                }),
                Err(e) => {
                    warn!("skipping corrupted embedding for entry {entry_id}: {e}");
                }
            }
        }

        // Store in cache with current generation (unfiltered so it can be
        // reused with different excludes).
        let shared = Arc::new(entries);
        *self
            .embedding_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((current_gen, Arc::clone(&shared)));

        Ok(shared
            .iter()
            .filter(|e| !exclude.contains(&e.source_key))
            .cloned()
            .collect())
    }
}
