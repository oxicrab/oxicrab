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
    let restored = deserialize_embedding(&bytes).unwrap();
    assert_eq!(original, restored);
}

#[test]
fn test_serialize_empty() {
    let v: Vec<f32> = vec![];
    let bytes = serialize_embedding(&v);
    assert!(bytes.is_empty());
    let restored = deserialize_embedding(&bytes).unwrap();
    assert!(restored.is_empty());
}

#[test]
fn test_deserialize_corrupt_blob() {
    // 5 bytes — not a multiple of 4
    let bad = vec![0u8, 1, 2, 3, 4];
    assert!(deserialize_embedding(&bad).is_err());
}

#[test]
fn test_deserialize_truncated_blob() {
    let good = serialize_embedding(&[1.0, 2.0]);
    // Truncate to 7 bytes
    let truncated = &good[..7];
    assert!(deserialize_embedding(truncated).is_err());
}

#[test]
fn test_default_cache_size_is_10k() {
    assert_eq!(DEFAULT_CACHE_SIZE, 10_000);
}

#[test]
fn test_embedding_cache_size_config_default() {
    let config = crate::config::MemoryConfig::default();
    assert_eq!(config.embedding_cache_size, 10_000);
}

#[test]
fn test_embedding_cache_size_config_serde() {
    let json = r#"{"embeddingCacheSize": 5000}"#;
    let config: crate::config::MemoryConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.embedding_cache_size, 5000);
}
