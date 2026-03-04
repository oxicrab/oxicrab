use super::*;

#[test]
fn test_is_daily_note_key() {
    assert!(is_daily_note_key("2026-02-22.md"));
    assert!(is_daily_note_key("2025-12-31.md"));
    assert!(!is_daily_note_key("MEMORY.md"));
    assert!(!is_daily_note_key("notes.md"));
    assert!(!is_daily_note_key("2026-02-22.txt"));
    assert!(!is_daily_note_key("2026-02-22"));
    assert!(!is_daily_note_key(""));
}

#[cfg(feature = "embeddings")]
#[test]
fn test_with_config_wires_fusion_strategy() {
    let tmp = tempfile::TempDir::new().unwrap();
    let memory_dir = tmp.path().join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();

    let mut config = crate::config::MemoryConfig::default();
    config.fusion_strategy = crate::config::FusionStrategy::Rrf;
    config.rrf_k = 42;
    config.hybrid_weight = 0.3;

    let store =
        MemoryStore::with_config(tmp.path(), 0, &config, std::collections::HashMap::new()).unwrap();
    assert_eq!(store.fusion_strategy, crate::config::FusionStrategy::Rrf);
    assert_eq!(store.rrf_k, 42);
    assert!((store.hybrid_weight - 0.3).abs() < f32::EPSILON);
}

#[tokio::test]
async fn test_group_memory_context_excludes_personal() {
    let tmp = tempfile::TempDir::new().unwrap();
    let memory_dir = tmp.path().join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(memory_dir.join("MEMORY.md"), "personal secret data").unwrap();

    let store = MemoryStore::with_indexer_interval(tmp.path(), 0).unwrap();

    // Normal mode includes MEMORY.md
    let normal = store.get_memory_context(None).unwrap();
    assert!(
        normal.contains("personal secret data"),
        "DM context should include MEMORY.md"
    );

    // Group mode excludes MEMORY.md
    let group = store.get_memory_context_scoped(None, true).unwrap();
    assert!(
        !group.contains("personal secret data"),
        "group context should NOT include MEMORY.md"
    );
}
