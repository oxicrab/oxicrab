/// Memory hygiene: archive old daily notes, purge expired archives, clean orphaned DB entries.
use crate::agent::memory::MemoryDB;
use anyhow::Result;
use chrono::{NaiveDate, Utc};
use std::path::Path;
use tracing::{debug, info, warn};

/// Archive daily notes older than `archive_after_days` into `memory/archive/`.
pub fn archive_old_notes(memory_dir: &Path, archive_after_days: u32) -> Result<u32> {
    if archive_after_days == 0 {
        return Ok(0);
    }

    let archive_dir = memory_dir.join("archive");
    let cutoff = Utc::now().date_naive() - chrono::Duration::days(i64::from(archive_after_days));
    let mut count = 0;

    for entry in std::fs::read_dir(memory_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if ext != "md" {
            continue;
        }

        // Only archive dated files (YYYY-MM-DD.md)
        let Ok(date) = NaiveDate::parse_from_str(stem, "%Y-%m-%d") else {
            continue;
        };

        if date < cutoff {
            std::fs::create_dir_all(&archive_dir)?;
            let dest = archive_dir.join(entry.file_name());
            std::fs::rename(&path, &dest)?;
            debug!("archived memory note: {}", stem);
            count += 1;
        }
    }

    if count > 0 {
        info!("archived {} old memory notes", count);
    }
    Ok(count)
}

/// Purge archived notes older than `purge_after_days`.
pub fn purge_expired_archives(memory_dir: &Path, purge_after_days: u32) -> Result<u32> {
    if purge_after_days == 0 {
        return Ok(0);
    }

    let archive_dir = memory_dir.join("archive");
    if !archive_dir.is_dir() {
        return Ok(0);
    }

    let cutoff = Utc::now().date_naive() - chrono::Duration::days(i64::from(purge_after_days));
    let mut count = 0;

    for entry in std::fs::read_dir(&archive_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(date) = NaiveDate::parse_from_str(stem, "%Y-%m-%d") else {
            continue;
        };

        if date < cutoff {
            std::fs::remove_file(&path)?;
            debug!("purged archived memory note: {}", stem);
            count += 1;
        }
    }

    if count > 0 {
        info!("purged {} expired archived notes", count);
    }
    Ok(count)
}

/// Remove DB entries whose source files no longer exist on disk.
pub fn cleanup_orphaned_entries(db: &MemoryDB, memory_dir: &Path) -> Result<u32> {
    let source_keys = db.list_source_keys()?;
    let mut count = 0;

    for key in source_keys {
        // Check both memory dir and archive dir
        let primary = memory_dir.join(&key);
        let archived = memory_dir.join("archive").join(&key);
        if !primary.exists() && !archived.exists() {
            db.remove_source(&key)?;
            debug!("cleaned orphaned memory entry: {}", key);
            count += 1;
        }
    }

    if count > 0 {
        info!("cleaned {} orphaned memory entries", count);
    }
    Ok(count)
}

/// Run all hygiene tasks.
pub fn run_hygiene(db: &MemoryDB, memory_dir: &Path, archive_days: u32, purge_days: u32) {
    if let Err(e) = archive_old_notes(memory_dir, archive_days) {
        warn!("memory archive failed: {}", e);
    }
    if let Err(e) = purge_expired_archives(memory_dir, purge_days) {
        warn!("memory purge failed: {}", e);
    }
    if let Err(e) = cleanup_orphaned_entries(db, memory_dir) {
        warn!("memory orphan cleanup failed: {}", e);
    }
}

#[cfg(test)]
mod tests {
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
}
