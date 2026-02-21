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
