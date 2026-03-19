use super::{MemoryDB, hash_text};
use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

impl MemoryDB {
    /// Insert a single memory entry directly into the DB (no backing file required).
    /// Empty or whitespace-only content is silently ignored.
    ///
    /// NOTE: Memory entries are not user-scoped. In a multi-user deployment,
    /// all users share the same memory pool. This is by design for a single-user
    /// personal agent. Multi-tenant isolation would require adding a scope/owner
    /// column and filtering on it in all search queries.
    pub fn insert_memory(&self, source_key: &str, content: &str) -> Result<()> {
        if content.trim().is_empty() {
            return Ok(());
        }
        let now = Utc::now().to_rfc3339();
        let hash = hash_text(content);
        let mut conn = self.lock_conn()?;
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
        self.invalidate_embedding_cache();
        Ok(())
    }

    /// Purge memory entries older than `days`, keeping `knowledge:` prefixed sources.
    /// Also cleans up orphaned embeddings. Returns number of entries deleted.
    pub fn purge_old_memory_entries(&self, days: u32) -> Result<usize> {
        if days == 0 {
            return Ok(0);
        }
        let mut conn = self.lock_conn()?;
        let cutoff = Utc::now() - chrono::Duration::days(i64::from(days));
        let cutoff_str = cutoff.to_rfc3339();
        let tx = conn.transaction()?;
        let deleted = tx.execute(
            "DELETE FROM memory_entries WHERE source_key NOT LIKE 'knowledge:%' AND created_at < ?",
            params![cutoff_str],
        )?;
        if deleted > 0 {
            // Clean up orphaned embeddings
            tx.execute(
                "DELETE FROM memory_embeddings WHERE entry_id NOT IN (SELECT id FROM memory_entries)",
                [],
            )?;
            // Clean up orphaned memory_sources with no remaining entries
            tx.execute(
                "DELETE FROM memory_sources WHERE source_key NOT IN (SELECT DISTINCT source_key FROM memory_entries)",
                [],
            )?;
        }
        tx.commit()?;
        if deleted > 0 {
            self.invalidate_embedding_cache();
        }
        Ok(deleted)
    }

    /// Get recent entries for a source key (for deduplication).
    pub fn get_recent_entries(&self, source_key: &str, limit: usize) -> Result<Vec<String>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT content FROM memory_entries WHERE source_key = ? ORDER BY created_at DESC LIMIT ?",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map(rusqlite::params![source_key, limit], |row| row.get(0))?
            .collect();
        rows.map_err(|e| anyhow::anyhow!("failed to get recent entries: {e}"))
    }

    /// Get recent entries across all daily source keys (for deduplication).
    pub fn get_recent_daily_entries(&self, limit: usize) -> Result<Vec<String>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT content FROM memory_entries WHERE source_key LIKE 'daily:%' ORDER BY created_at DESC LIMIT ?",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map(rusqlite::params![limit], |row| row.get(0))?
            .collect();
        rows.map_err(|e| anyhow::anyhow!("failed to get recent daily entries: {e}"))
    }

    /// Delete all memory entries for a given source key.
    /// Also removes associated embeddings and the source record.
    /// Returns number of entries deleted.
    pub fn delete_by_source_key(&self, source_key: &str) -> Result<usize> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        // Delete embeddings first (foreign key)
        tx.execute(
            "DELETE FROM memory_embeddings WHERE entry_id IN (SELECT id FROM memory_entries WHERE source_key = ?)",
            params![source_key],
        )?;
        let deleted = tx.execute(
            "DELETE FROM memory_entries WHERE source_key = ?",
            params![source_key],
        )?;
        tx.execute(
            "DELETE FROM memory_sources WHERE source_key = ?",
            params![source_key],
        )?;
        tx.commit()?;
        if deleted > 0 {
            self.invalidate_embedding_cache();
        }
        Ok(deleted)
    }

    /// List all source keys with their entry counts.
    pub fn list_sources_with_counts(&self) -> Result<Vec<(String, usize)>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT source_key, COUNT(*) as cnt FROM memory_entries GROUP BY source_key ORDER BY source_key",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
            })?
            .collect();
        rows.map_err(|e| anyhow::anyhow!("failed to list sources: {e}"))
    }
}
