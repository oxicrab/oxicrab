use super::*;

#[cfg(feature = "embeddings")]
#[test]
fn test_with_config_wires_fusion_strategy() {
    let tmp = tempfile::TempDir::new().unwrap();

    let mut config = crate::config::MemoryConfig::default();
    config.fusion_strategy = crate::config::FusionStrategy::Rrf;
    config.rrf_k = 42;
    config.hybrid_weight = 0.3;

    let store = MemoryStore::with_config(tmp.path(), &config).unwrap();
    assert_eq!(store.fusion_strategy, crate::config::FusionStrategy::Rrf);
    assert_eq!(store.rrf_k, 42);
    assert!((store.hybrid_weight - 0.3).abs() < f32::EPSILON);
}

#[test]
fn test_append_today_inserts_to_db() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = MemoryStore::new(tmp.path()).unwrap();

    store
        .append_today("Test fact: user likes Rust")
        .expect("append daily note");

    let entries = store.get_recent_daily_entries(10).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("user likes Rust"));
}

#[test]
fn test_append_today_multiple() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = MemoryStore::new(tmp.path()).unwrap();

    store.append_today("First fact").unwrap();
    store.append_today("Second fact").unwrap();

    let entries = store.get_recent_daily_entries(10).unwrap();
    assert_eq!(entries.len(), 2);
}

#[test]
fn test_append_to_section_and_read() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = MemoryStore::new(tmp.path()).unwrap();

    store.append_to_section("Facts", "user likes Rust").unwrap();
    store
        .append_to_section("Facts", "user prefers dark mode")
        .unwrap();

    let section = store.read_today_section("Facts").unwrap();
    assert!(section.contains("user likes Rust"));
    assert!(section.contains("user prefers dark mode"));
}

#[test]
fn test_group_memory_context_excludes_daily() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = MemoryStore::new(tmp.path()).unwrap();

    store.append_today("personal secret data").unwrap();

    // Group mode should not include daily notes in search
    let group = store
        .get_memory_context_scoped(Some("personal secret"), true)
        .unwrap();
    assert!(
        !group.contains("personal secret data"),
        "group context should NOT include daily notes"
    );
}
