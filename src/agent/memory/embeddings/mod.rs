/// Local embedding generation via fastembed (ONNX-based, no API key needed).
use std::sync::Mutex;

use anyhow::Result;
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use tracing::info;

pub struct EmbeddingService {
    model: Mutex<TextEmbedding>,
}

impl EmbeddingService {
    /// Load embedding model. This downloads the model on first use (~30MB).
    pub fn new(model_name: &str) -> Result<Self> {
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
        info!("embedding model loaded: {}", model_name);
        Ok(Self {
            model: Mutex::new(model),
        })
    }

    /// Embed multiple texts (batch). Returns one vector per text.
    pub fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let docs: Vec<String> = texts.iter().map(std::string::ToString::to_string).collect();
        let mut model = self.model.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let embeddings = model.embed(docs, None)?;
        Ok(embeddings)
    }

    /// Embed a single query string.
    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        let mut model = self.model.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let embeddings = model.embed(vec![query.to_string()], None)?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("empty embedding result"))
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
pub fn deserialize_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| {
            let arr: [u8; 4] = chunk.try_into().expect("chunk is exactly 4 bytes");
            f32::from_le_bytes(arr)
        })
        .collect()
}

#[cfg(test)]
mod tests;
