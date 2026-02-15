/// Local embedding generation via fastembed (ONNX-based, no API key needed).
use anyhow::Result;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use tracing::info;

pub struct EmbeddingService {
    model: TextEmbedding,
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

        let model =
            TextEmbedding::try_new(InitOptions::new(model_type).with_show_download_progress(true))?;
        info!("embedding model loaded: {}", model_name);
        Ok(Self { model })
    }

    /// Embed multiple texts (batch). Returns one vector per text.
    pub fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let docs: Vec<String> = texts.iter().map(std::string::ToString::to_string).collect();
        let embeddings = self.model.embed(docs, None)?;
        Ok(embeddings)
    }

    /// Embed a single query string.
    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        let embeddings = self.model.embed(vec![query.to_string()], None)?;
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
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&a, &b)).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_mismatched_length() {
        let a = vec![1.0, 0.0];
        let b = vec![1.0];
        assert!((cosine_similarity(&a, &b)).abs() < 1e-6);
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let original = vec![1.0_f32, -0.5, 0.0, 3.1, f32::MIN, f32::MAX];
        let bytes = serialize_embedding(&original);
        let restored = deserialize_embedding(&bytes);
        assert_eq!(original, restored);
    }

    #[test]
    fn test_serialize_empty() {
        let v: Vec<f32> = vec![];
        let bytes = serialize_embedding(&v);
        assert!(bytes.is_empty());
        let restored = deserialize_embedding(&bytes);
        assert!(restored.is_empty());
    }
}
