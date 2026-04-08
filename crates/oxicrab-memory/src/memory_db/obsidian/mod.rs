use super::MemoryDB;
use anyhow::Result;
use rusqlite::params;
use std::collections::HashMap;

/// Metadata for a cached file, mirroring the obsidian cache struct.
#[derive(Debug, Clone)]
pub struct ObsidianSyncRow {
    pub content_hash: String,
    pub last_synced_at: i64,
    pub size: u64,
}

/// A queued write row from the database, including the auto-generated id.
#[derive(Debug, Clone)]
pub struct ObsidianQueueRow {
    pub id: i64,
    pub path: String,
    pub content: String,
    pub operation: String,
    pub queued_at: i64,
    pub pre_write_hash: Option<String>,
}

/// Cached file metadata, matching the obsidian tool's `CachedFileMeta` struct.
/// Defined here to avoid circular dependencies between memory and tool crates.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedFileMeta {
    pub content_hash: String,
    pub last_synced_at: i64,
    pub size: u64,
}

/// A queued write operation, matching the obsidian tool's `QueuedWrite` struct.
/// Defined here to avoid circular dependencies between memory and tool crates.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueuedWrite {
    pub path: String,
    pub content: String,
    pub operation: String,
    pub queued_at: i64,
    pub pre_write_hash: Option<String>,
}

impl MemoryDB {
    /// Insert or replace a sync state entry for a single file in a vault.
    pub fn upsert_obsidian_sync(
        &self,
        vault_name: &str,
        file_path: &str,
        content_hash: &str,
        last_synced_at: i64,
        size: u64,
    ) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO obsidian_sync_state
             (vault_name, file_path, content_hash, last_synced_at, size)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                vault_name,
                file_path,
                content_hash,
                last_synced_at,
                size as i64
            ],
        )?;
        Ok(())
    }

    /// Look up sync state for a single file.
    pub fn get_obsidian_sync(
        &self,
        vault_name: &str,
        file_path: &str,
    ) -> Result<Option<ObsidianSyncRow>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT content_hash, last_synced_at, size
             FROM obsidian_sync_state WHERE vault_name = ?1 AND file_path = ?2",
        )?;
        let mut rows = stmt.query(params![vault_name, file_path])?;
        if let Some(row) = rows.next()? {
            Ok(Some(ObsidianSyncRow {
                content_hash: row.get(0)?,
                last_synced_at: row.get(1)?,
                size: row.get::<_, i64>(2)? as u64,
            }))
        } else {
            Ok(None)
        }
    }

    /// List all sync state entries for a vault.
    pub fn list_obsidian_sync(&self, vault_name: &str) -> Result<HashMap<String, ObsidianSyncRow>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT file_path, content_hash, last_synced_at, size
             FROM obsidian_sync_state WHERE vault_name = ?1",
        )?;
        let rows = stmt
            .query_map(params![vault_name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    ObsidianSyncRow {
                        content_hash: row.get(1)?,
                        last_synced_at: row.get(2)?,
                        size: row.get::<_, i64>(3)? as u64,
                    },
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows.into_iter().collect())
    }

    /// Remove a single file's sync state. Returns `true` if a row was deleted.
    pub fn remove_obsidian_sync(&self, vault_name: &str, file_path: &str) -> Result<bool> {
        let conn = self.lock_conn()?;
        let deleted = conn.execute(
            "DELETE FROM obsidian_sync_state WHERE vault_name = ?1 AND file_path = ?2",
            params![vault_name, file_path],
        )?;
        Ok(deleted > 0)
    }

    /// Clear all sync state for a vault. Returns count deleted.
    pub fn clear_obsidian_sync(&self, vault_name: &str) -> Result<usize> {
        let conn = self.lock_conn()?;
        let deleted = conn.execute(
            "DELETE FROM obsidian_sync_state WHERE vault_name = ?1",
            params![vault_name],
        )?;
        Ok(deleted)
    }

    /// Derive `last_full_sync_at` as the MIN of all `last_synced_at` for the vault,
    /// or 0 if no entries exist.
    pub fn get_last_full_sync(&self, vault_name: &str) -> Result<i64> {
        let conn = self.lock_conn()?;
        let min: Option<i64> = conn.query_row(
            "SELECT MIN(last_synced_at) FROM obsidian_sync_state WHERE vault_name = ?1",
            params![vault_name],
            |row| row.get(0),
        )?;
        Ok(min.unwrap_or(0))
    }

    /// Add a write to the queue. Returns the auto-generated row id.
    pub fn add_obsidian_queue(
        &self,
        vault_name: &str,
        path: &str,
        content: &str,
        operation: &str,
        queued_at: i64,
        pre_write_hash: Option<&str>,
    ) -> Result<i64> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO obsidian_write_queue
             (vault_name, path, content, operation, queued_at, pre_write_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                vault_name,
                path,
                content,
                operation,
                queued_at,
                pre_write_hash
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// List all queued writes for a vault.
    pub fn list_obsidian_queue(&self, vault_name: &str) -> Result<Vec<ObsidianQueueRow>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, path, content, operation, queued_at, pre_write_hash
             FROM obsidian_write_queue WHERE vault_name = ?1 ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![vault_name], |row| {
                Ok(ObsidianQueueRow {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    content: row.get(2)?,
                    operation: row.get(3)?,
                    queued_at: row.get(4)?,
                    pre_write_hash: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Remove a single queued write by id. Returns `true` if a row was deleted.
    pub fn remove_obsidian_queue(&self, id: i64) -> Result<bool> {
        let conn = self.lock_conn()?;
        let deleted = conn.execute(
            "DELETE FROM obsidian_write_queue WHERE id = ?1",
            params![id],
        )?;
        Ok(deleted > 0)
    }

    /// Clear all queued writes for a vault. Returns count deleted.
    pub fn clear_obsidian_queue(&self, vault_name: &str) -> Result<usize> {
        let conn = self.lock_conn()?;
        let deleted = conn.execute(
            "DELETE FROM obsidian_write_queue WHERE vault_name = ?1",
            params![vault_name],
        )?;
        Ok(deleted)
    }

    /// Atomically replace all sync state for a vault (clear + re-insert in one transaction).
    pub fn replace_obsidian_sync(
        &self,
        vault_name: &str,
        files: &HashMap<String, CachedFileMeta>,
    ) -> Result<()> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM obsidian_sync_state WHERE vault_name = ?1",
            params![vault_name],
        )?;
        for (path, meta) in files {
            tx.execute(
                "INSERT OR REPLACE INTO obsidian_sync_state
                 (vault_name, file_path, content_hash, last_synced_at, size)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    vault_name,
                    path,
                    meta.content_hash,
                    meta.last_synced_at,
                    meta.size as i64
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Atomically replace all queued writes for a vault (clear + re-insert in one transaction).
    pub fn replace_obsidian_queue(&self, vault_name: &str, queue: &[QueuedWrite]) -> Result<()> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM obsidian_write_queue WHERE vault_name = ?1",
            params![vault_name],
        )?;
        for item in queue {
            tx.execute(
                "INSERT INTO obsidian_write_queue
                 (vault_name, path, content, operation, queued_at, pre_write_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    vault_name,
                    item.path,
                    item.content,
                    item.operation,
                    item.queued_at,
                    item.pre_write_hash
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Count queued writes for a vault.
    pub fn count_obsidian_queue(&self, vault_name: &str) -> Result<usize> {
        let conn = self.lock_conn()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM obsidian_write_queue WHERE vault_name = ?1",
            params![vault_name],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests;
