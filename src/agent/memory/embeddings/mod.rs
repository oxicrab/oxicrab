/// Local embedding generation via fastembed (ONNX-based, no API key needed).
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use lru::LruCache;
use tracing::{debug, info, warn};

const DEFAULT_CACHE_SIZE: usize = 10_000;

pub struct EmbeddingService {
    model: Mutex<TextEmbedding>,
    cache: Mutex<LruCache<String, Vec<f32>>>,
}

impl EmbeddingService {
    /// Load embedding model. This downloads the model on first use (~30MB).
    pub fn new(model_name: &str) -> Result<Self> {
        Self::with_cache_size(model_name, DEFAULT_CACHE_SIZE)
    }

    /// Load embedding model with a custom query embedding cache size.
    pub fn with_cache_size(model_name: &str, cache_size: usize) -> Result<Self> {
        let model_type = match model_name {
            "BAAI/bge-small-en-v1.5" => EmbeddingModel::BGESmallENV15,
            "BAAI/bge-base-en-v1.5" => EmbeddingModel::BGEBaseENV15,
            _ => {
                anyhow::bail!(
                    "unsupported embedding model '{}'; use BAAI/bge-small-en-v1.5 or BAAI/bge-base-en-v1.5",
                    model_name
                );
            }
        };

        let model = TextEmbedding::try_new(
            TextInitOptions::new(model_type).with_show_download_progress(true),
        )?;
        info!(
            "embedding model loaded: {} (cache_size={})",
            model_name, cache_size
        );

        let cap = NonZeroUsize::new(cache_size.max(1)).unwrap();
        Ok(Self {
            model: Mutex::new(model),
            cache: Mutex::new(LruCache::new(cap)),
        })
    }

    /// Embed multiple texts (batch). Returns one vector per text.
    /// These are not cached (used for indexing, where each text is unique).
    pub fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let docs: Vec<String> = texts.iter().map(std::string::ToString::to_string).collect();
        let mut model = self.model.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let embeddings = model.embed(docs, None)?;
        Ok(embeddings)
    }

    /// Embed a single query string. Results are cached in an LRU cache
    /// to avoid redundant ONNX inference for repeated queries.
    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        // Check cache first
        {
            let mut cache = self.cache.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            if let Some(cached) = cache.get(query) {
                debug!("embedding cache hit for query (len={})", query.len());
                return Ok(cached.clone());
            }
        }

        // Cache miss — compute embedding
        let embedding = {
            let mut model = self.model.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            let embeddings = model.embed(vec![query.to_string()], None)?;
            embeddings
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("empty embedding result"))?
        };

        // Store in cache
        {
            let mut cache = self.cache.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            cache.put(query.to_string(), embedding.clone());
        }

        Ok(embedding)
    }

    /// Number of entries currently in the query embedding cache.
    #[cfg(test)]
    pub fn cache_len(&self) -> usize {
        self.cache.lock().map_or(0, |c| c.len())
    }
}

/// Lazy wrapper that initializes the embedding model in a background task.
/// Callers can check readiness via `get()` or `is_ready()`.
pub struct LazyEmbeddingService {
    cell: Arc<tokio::sync::OnceCell<EmbeddingService>>,
}

impl LazyEmbeddingService {
    /// Spawn background initialization of the embedding model.
    pub fn new(model_name: String, cache_size: usize) -> Self {
        let cell = Arc::new(tokio::sync::OnceCell::new());
        let cell_clone = cell.clone();
        tokio::spawn(async move {
            match tokio::task::spawn_blocking(move || {
                EmbeddingService::with_cache_size(&model_name, cache_size)
            })
            .await
            {
                Ok(Ok(svc)) => {
                    let _ = cell_clone.set(svc);
                    info!("embedding model initialized (background)");
                }
                Ok(Err(e)) => warn!("embedding init failed: {}", e),
                Err(e) => warn!("embedding init panicked: {}", e),
            }
        });
        Self { cell }
    }

    /// Get the service if ready, None if still initializing.
    pub fn get(&self) -> Option<&EmbeddingService> {
        self.cell.get()
    }

    /// Check if initialization is complete.
    pub fn is_ready(&self) -> bool {
        self.cell.get().is_some()
    }
}

/// Cosine similarity between two vectors. fastembed produces normalized vectors,
/// so dot product equals cosine similarity.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Serialize an embedding vector to little-endian bytes for `SQLite` BLOB storage.
pub fn serialize_embedding(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for &val in v {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// Deserialize an embedding from little-endian bytes.
///
/// Returns an error if the byte slice length is not a multiple of 4
/// (indicating corruption or truncation).
pub fn deserialize_embedding(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        anyhow::bail!(
            "invalid embedding blob: {} bytes (not a multiple of 4)",
            bytes.len()
        );
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| {
            let arr: [u8; 4] = chunk.try_into().unwrap();
            f32::from_le_bytes(arr)
        })
        .collect())
}

#[cfg(test)]
mod tests;
