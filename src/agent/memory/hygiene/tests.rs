use super::*;
use tempfile::TempDir;

fn create_dated_file(dir: &Path, date_str: &str, content: &str) {
    let path = dir.join(format!("{}.md", date_str));
    std::fs::write(path, content).unwrap();
}

#[test]
fn test_archive_old_notes() {
    let tmp = TempDir::new().unwrap();
    let memory_dir = tmp.path();

    // Create files: one old, one recent
    create_dated_file(memory_dir, "2020-01-01", "old note");
    create_dated_file(memory_dir, "2099-12-31", "future note");
    // Non-dated file should be ignored
    std::fs::write(memory_dir.join("MEMORY.md"), "long-term").unwrap();

    let count = archive_old_notes(memory_dir, 30).unwrap();
    assert_eq!(count, 1);
    assert!(!memory_dir.join("2020-01-01.md").exists());
    assert!(memory_dir.join("archive/2020-01-01.md").exists());
    assert!(memory_dir.join("2099-12-31.md").exists());
    assert!(memory_dir.join("MEMORY.md").exists());
}

#[test]
fn test_archive_zero_days_is_noop() {
    let tmp = TempDir::new().unwrap();
    create_dated_file(tmp.path(), "2020-01-01", "old note");
    let count = archive_old_notes(tmp.path(), 0).unwrap();
    assert_eq!(count, 0);
    assert!(tmp.path().join("2020-01-01.md").exists());
}

#[test]
fn test_purge_expired_archives() {
    let tmp = TempDir::new().unwrap();
    let archive_dir = tmp.path().join("archive");
    std::fs::create_dir(&archive_dir).unwrap();

    create_dated_file(&archive_dir, "2020-01-01", "very old");
    create_dated_file(&archive_dir, "2099-12-31", "future");

    let count = purge_expired_archives(tmp.path(), 90).unwrap();
    assert_eq!(count, 1);
    assert!(!archive_dir.join("2020-01-01.md").exists());
    assert!(archive_dir.join("2099-12-31.md").exists());
}

#[test]
fn test_purge_no_archive_dir() {
    let tmp = TempDir::new().unwrap();
    let count = purge_expired_archives(tmp.path(), 90).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_cleanup_orphaned_entries() {
    let tmp = TempDir::new().unwrap();
    let memory_dir = tmp.path().join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();

    let db_path = tmp.path().join("test.db");
    let db = MemoryDB::new(&db_path).unwrap();

    // Index a file then delete it
    let f = memory_dir.join("notes.md");
    std::fs::write(&f, "This is a test file about orphaned entries.").unwrap();
    db.index_file("notes.md", &f).unwrap();
    std::fs::remove_file(&f).unwrap();

    let count = cleanup_orphaned_entries(&db, &memory_dir).unwrap();
    assert_eq!(count, 1);

    // Search should return nothing now
    let results = db.search("orphaned", 10, None).unwrap();
    assert!(results.is_empty());
}
