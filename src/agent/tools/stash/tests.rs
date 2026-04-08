use super::*;

#[tokio::test]
async fn test_stash_and_retrieve() {
    let stash = ToolOutputStash::new();
    let content = "hello world".repeat(100);
    let key = stash.stash(content.clone()).await.unwrap();
    assert!(key.starts_with("stash_"));

    let (chunk, total) = stash.retrieve(&key, 0, 50000).await.unwrap();
    assert_eq!(total, content.len());
    assert_eq!(chunk, content);
}

#[tokio::test]
async fn test_retrieve_with_offset() {
    let stash = ToolOutputStash::new();
    let content = "abcdefghij".repeat(10); // 100 chars
    let key = stash.stash(content.clone()).await.unwrap();

    let (chunk, total) = stash.retrieve(&key, 50, 20).await.unwrap();
    assert_eq!(total, 100);
    assert_eq!(chunk.len(), 20);
    assert_eq!(chunk, &content[50..70]);
}

#[tokio::test]
async fn test_retrieve_past_end() {
    let stash = ToolOutputStash::new();
    let key = stash.stash("short".to_string()).await.unwrap();

    let (chunk, total) = stash.retrieve(&key, 100, 50).await.unwrap();
    assert_eq!(total, 5);
    assert!(chunk.is_empty());
}

#[tokio::test]
async fn test_oversized_returns_none() {
    let stash = ToolOutputStash::new();
    // 32 MB + 1 byte exceeds the default budget
    let oversized = "x".repeat(DEFAULT_MAX_TOTAL_BYTES + 1);
    assert!(stash.stash(oversized).await.is_none());
}

#[tokio::test]
async fn test_eviction_by_count() {
    let stash = ToolOutputStash::new();
    let mut keys = Vec::new();
    for i in 0..33 {
        keys.push(stash.stash(format!("entry_{i}")).await.unwrap());
    }
    // First entry should be evicted (32 max)
    assert!(stash.retrieve(&keys[0], 0, 100).await.is_none());
    // Last entry should exist
    assert!(stash.retrieve(&keys[32], 0, 100).await.is_some());
}

#[tokio::test]
async fn test_not_found() {
    let stash = ToolOutputStash::new();
    assert!(stash.retrieve("nonexistent", 0, 100).await.is_none());
}
