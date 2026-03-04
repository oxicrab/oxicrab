use super::{MemoryDB, hash_text, split_into_chunks, strip_html_tags};
use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use std::path::Path;
use tracing::debug;

impl MemoryDB {
    fn get_mtime_ms(path: &Path) -> i64 {
        path.metadata()
            .and_then(|m| {
                m.modified().map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis().min(i64::MAX as u128) as i64)
                })
            })
            .unwrap_or(0)
    }

    pub fn index_file(&self, source_key: &str, path: &Path) -> Result<()> {
        let text = if path.exists() && path.is_file() {
            std::fs::read_to_string(path).unwrap_or_default()
        } else {
            String::new()
        };
        self.index_text(source_key, path, &text)
    }

    /// Shared indexing: mtime check, wipe old entries, chunk text, insert.
    fn index_text(&self, source_key: &str, path: &Path, text: &str) -> Result<()> {
        let mtime_ms = Self::get_mtime_ms(path);
        let now = Utc::now().to_rfc3339();

        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;

        // Check if unchanged
        let existing: Option<i64> = conn
            .query_row(
                "SELECT mtime_ns FROM memory_sources WHERE source_key = ?",
                [source_key],
                |row| row.get(0),
            )
            .ok();

        if let Some(existing_mtime) = existing
            && existing_mtime == mtime_ms
        {
            return Ok(()); // unchanged
        }

        // Wrap delete-insert-update in a transaction for atomicity.
        // Uses rusqlite's Transaction API so the transaction auto-rolls-back
        // on drop if not committed, preventing a stuck open transaction on
        // COMMIT failure.
        let tx = conn.transaction()?;
        // Wipe old entries
        tx.execute(
            "DELETE FROM memory_entries WHERE source_key = ?",
            [source_key],
        )?;

        for chunk in split_into_chunks(text) {
            let hash = hash_text(&chunk);
            tx.execute(
                "INSERT OR IGNORE INTO memory_entries
                    (source_key, content, content_hash, created_at)
                VALUES (?, ?, ?, ?)",
                params![source_key, chunk, hash, now],
            )?;
        }

        // Update source record
        tx.execute(
            "INSERT INTO memory_sources (source_key, mtime_ns, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(source_key)
            DO UPDATE SET mtime_ns = excluded.mtime_ns,
                          updated_at = excluded.updated_at",
            params![source_key, mtime_ms, now],
        )?;
        tx.commit()?;

        // Invalidate embedding cache since entries changed
        if let Ok(mut cache) = self.embedding_cache.lock() {
            *cache = None;
        }

        debug!("indexed {}", source_key);
        Ok(())
    }

    pub fn index_directory(&self, memory_dir: &Path) -> Result<()> {
        if !memory_dir.is_dir() {
            return Ok(());
        }

        for entry in std::fs::read_dir(memory_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension() == Some(std::ffi::OsStr::new("md"))
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                self.index_file(name, &path)?;
            }
        }

        Ok(())
    }

    /// Index all supported files in a knowledge directory.
    ///
    /// Accepts `.md`, `.txt`, and `.html` files. Source keys are prefixed
    /// with `knowledge:` to distinguish from memory notes. HTML files have
    /// tags stripped before chunking.
    pub fn index_knowledge_directory(&self, knowledge_dir: &Path) -> Result<()> {
        if !knowledge_dir.is_dir() {
            return Ok(());
        }

        let mut indexed_keys = std::collections::HashSet::new();

        for entry in walkdir::WalkDir::new(knowledge_dir)
            .into_iter()
            .filter_map(std::result::Result::ok)
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if !matches!(ext.as_str(), "md" | "txt" | "html") {
                continue;
            }
            let Some(rel) = path.strip_prefix(knowledge_dir).ok() else {
                continue;
            };
            let Some(rel_str) = rel.to_str() else {
                continue;
            };
            let source_key = format!("knowledge:{rel_str}");
            if ext == "html" {
                self.index_html_file(&source_key, path)?;
            } else {
                self.index_file(&source_key, path)?;
            }
            indexed_keys.insert(source_key);
        }

        // Remove orphaned knowledge entries that no longer have files on disk
        let all_keys = self.list_source_keys()?;
        for key in all_keys {
            if key.starts_with("knowledge:") && !indexed_keys.contains(&key) {
                self.remove_source(&key)?;
                debug!("removed orphaned knowledge entry: {}", key);
            }
        }

        Ok(())
    }

    /// Index an HTML file by stripping tags before chunking.
    fn index_html_file(&self, source_key: &str, path: &Path) -> Result<()> {
        let html = if path.exists() && path.is_file() {
            std::fs::read_to_string(path).unwrap_or_default()
        } else {
            String::new()
        };
        let text = strip_html_tags(&html);
        self.index_text(source_key, path, &text)
    }
}
