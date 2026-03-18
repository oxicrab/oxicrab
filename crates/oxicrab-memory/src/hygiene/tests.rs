use super::*;
use tempfile::TempDir;

#[test]
fn test_run_hygiene_purges_search_logs() {
    let tmp = TempDir::new().unwrap();
    let db = MemoryDB::new(tmp.path().join("test.db")).unwrap();
    // Should not panic on an empty DB
    run_hygiene(&db, 90);
}

#[test]
fn test_cleanup_workspace_files_removes_expired() {
    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().to_path_buf();
    let memory_dir = workspace_root.join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let db = MemoryDB::new(memory_dir.join("memory.sqlite3")).unwrap();

    // Create an old temp file
    let old_file = workspace_root.join("temp/2026-02-10/old.txt");
    std::fs::create_dir_all(old_file.parent().unwrap()).unwrap();
    std::fs::write(&old_file, "old content").unwrap();

    // Register with old date (use the test-only method)
    db.register_workspace_file_with_date(
        "temp/2026-02-10/old.txt",
        "temp",
        Some("old.txt"),
        11,
        None,
        None,
        "2026-02-10T00:00:00Z",
    )
    .unwrap();

    // Create a recent file that should NOT be removed
    let new_file = workspace_root.join("temp/2026-02-27/new.txt");
    std::fs::create_dir_all(new_file.parent().unwrap()).unwrap();
    std::fs::write(&new_file, "new content").unwrap();
    db.register_workspace_file(
        "temp/2026-02-27/new.txt",
        "temp",
        Some("new.txt"),
        11,
        None,
        None,
    )
    .unwrap();

    let mut ttl = std::collections::HashMap::new();
    ttl.insert("temp".to_string(), Some(7u64));
    ttl.insert("code".to_string(), None);

    let removed = cleanup_workspace_files(&db, &workspace_root, &ttl).unwrap();
    assert_eq!(removed, 1);
    assert!(!old_file.exists());
    assert!(new_file.exists());
}

#[test]
fn test_cleanup_workspace_files_skips_no_ttl_categories() {
    let tmp = TempDir::new().unwrap();
    let memory_dir = tmp.path().join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let db = MemoryDB::new(memory_dir.join("memory.sqlite3")).unwrap();

    db.register_workspace_file_with_date(
        "code/2025-01-01/ancient.py",
        "code",
        Some("ancient.py"),
        100,
        None,
        None,
        "2025-01-01T00:00:00Z",
    )
    .unwrap();

    let mut ttl = std::collections::HashMap::new();
    ttl.insert("code".to_string(), None); // No TTL for code

    let removed = cleanup_workspace_files(&db, tmp.path(), &ttl).unwrap();
    assert_eq!(removed, 0);
}

#[test]
fn test_cleanup_workspace_files_handles_missing_file() {
    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().to_path_buf();
    let memory_dir = workspace_root.join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let db = MemoryDB::new(memory_dir.join("memory.sqlite3")).unwrap();

    // Register an old file but don't create it on disk — should still unregister
    db.register_workspace_file_with_date(
        "temp/2025-01-01/gone.txt",
        "temp",
        Some("gone.txt"),
        50,
        None,
        None,
        "2025-01-01T00:00:00Z",
    )
    .unwrap();

    let mut ttl = std::collections::HashMap::new();
    ttl.insert("temp".to_string(), Some(7u64));

    let removed = cleanup_workspace_files(&db, &workspace_root, &ttl).unwrap();
    assert_eq!(removed, 1);

    // Verify it was unregistered
    let remaining = db.list_workspace_files(Some("temp"), None, None).unwrap();
    assert!(remaining.is_empty());
}
