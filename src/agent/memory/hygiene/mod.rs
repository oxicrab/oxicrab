/// Memory hygiene: archive old daily notes, purge expired archives, clean orphaned DB entries.
use crate::agent::memory::MemoryDB;
use anyhow::Result;
use chrono::{NaiveDate, Utc};
use std::path::Path;
use tracing::{debug, info, warn};

/// Acquire an exclusive lock on the memory directory for hygiene operations.
/// This prevents races with `append_today()` and `get_memory_context()` reads.
fn lock_memory_exclusive(memory_dir: &Path) -> Option<std::fs::File> {
    let lock_path = memory_dir.join(".hygiene.lock");
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&lock_path)
        .ok()?;
    fs2::FileExt::lock_exclusive(&lock_file).ok()?;
    Some(lock_file)
}

/// Archive daily notes older than `archive_after_days` into `memory/archive/`.
pub fn archive_old_notes(memory_dir: &Path, archive_after_days: u32) -> Result<u32> {
    if archive_after_days == 0 {
        return Ok(0);
    }

    let _lock = lock_memory_exclusive(memory_dir);

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

    let _lock = lock_memory_exclusive(memory_dir);

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
mod tests;
