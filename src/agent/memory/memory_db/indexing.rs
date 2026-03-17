use super::{MemoryDB, hash_text};
use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

impl MemoryDB {
    /// Insert a single memory entry directly into the DB (no backing file required).
    /// Empty or whitespace-only content is silently ignored.
    pub fn insert_memory(&self, source_key: &str, content: &str) -> Result<()> {
        if content.trim().is_empty() {
            return Ok(());
        }
        let now = Utc::now().to_rfc3339();
        let hash = hash_text(content);
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO memory_entries (source_key, content, content_hash, created_at) VALUES (?, ?, ?, ?)",
            params![source_key, content, hash, now],
        )?;
        tx.execute(
            "INSERT INTO memory_sources (source_key, mtime_ns, updated_at) VALUES (?, 0, ?) ON CONFLICT(source_key) DO UPDATE SET updated_at = excluded.updated_at",
            params![source_key, now],
        )?;
        tx.commit()?;
        // Invalidate embedding cache
        self.embedding_generation
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        self.embedding_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Ok(())
    }

    /// Get recent entries for a source key (for deduplication).
    pub fn get_recent_entries(&self, source_key: &str, limit: usize) -> Result<Vec<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT content FROM memory_entries WHERE source_key = ? ORDER BY created_at DESC LIMIT ?",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map(rusqlite::params![source_key, limit], |row| row.get(0))?
            .collect();
        rows.map_err(|e| anyhow::anyhow!("failed to get recent entries: {e}"))
    }
}
