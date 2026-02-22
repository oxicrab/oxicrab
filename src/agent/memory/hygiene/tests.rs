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

    let count = archive_old_notes(memory_dir, 30, None).unwrap();
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
    let count = archive_old_notes(tmp.path(), 0, None).unwrap();
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

#[test]
fn test_utility_based_early_archive() {
    let tmp = TempDir::new().unwrap();
    let memory_dir = tmp.path();
    let db_path = tmp.path().join("test.db");
    let db = MemoryDB::new(&db_path).unwrap();

    // archive_after_days = 30, so early_cutoff = 15 days ago
    // Create a note 20 days ago (between early and normal cutoff)
    let twenty_days_ago = (Utc::now().date_naive() - chrono::Duration::days(20))
        .format("%Y-%m-%d")
        .to_string();
    let note_content = "This is a note about utility based archiving and early pruning.";
    create_dated_file(memory_dir, &twenty_days_ago, note_content);

    // Index the file so it exists in DB
    let note_path = memory_dir.join(format!("{}.md", twenty_days_ago));
    db.index_file(&format!("{}.md", twenty_days_ago), &note_path)
        .unwrap();

    // Without db: note should NOT be early-archived (it's between early and normal cutoff)
    let count = archive_old_notes(memory_dir, 30, None).unwrap();
    assert_eq!(count, 0, "should not archive without db for utility check");

    // With db but note has zero hits: should be early-archived
    let count = archive_old_notes(memory_dir, 30, Some(&db)).unwrap();
    assert_eq!(count, 1, "should early-archive unused note");
    assert!(!note_path.exists());
    assert!(
        memory_dir
            .join("archive")
            .join(format!("{}.md", twenty_days_ago))
            .exists()
    );
}

#[test]
fn test_utility_based_keeps_used_notes() {
    let tmp = TempDir::new().unwrap();
    let memory_dir = tmp.path();
    let db_path = tmp.path().join("test.db");
    let db = MemoryDB::new(&db_path).unwrap();

    // Create a note 20 days ago (between early cutoff at 15 and normal at 30)
    let twenty_days_ago = (Utc::now().date_naive() - chrono::Duration::days(20))
        .format("%Y-%m-%d")
        .to_string();
    let note_content = "This is about Rust programming and memory management techniques.";
    create_dated_file(memory_dir, &twenty_days_ago, note_content);

    let note_path = memory_dir.join(format!("{}.md", twenty_days_ago));
    db.index_file(&format!("{}.md", twenty_days_ago), &note_path)
        .unwrap();

    // Search for something that will hit this note — this creates search hits
    let _ = db.search("Rust programming", 10, None).unwrap();

    // With db: note has search hits, should NOT be early-archived
    let count = archive_old_notes(memory_dir, 30, Some(&db)).unwrap();
    assert_eq!(count, 0, "should keep note that has search hits");
    assert!(note_path.exists());
}
