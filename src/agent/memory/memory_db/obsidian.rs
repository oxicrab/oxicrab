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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let deleted = conn.execute(
            "DELETE FROM obsidian_sync_state WHERE vault_name = ?1 AND file_path = ?2",
            params![vault_name, file_path],
        )?;
        Ok(deleted > 0)
    }

    /// Clear all sync state for a vault. Returns count deleted.
    pub fn clear_obsidian_sync(&self, vault_name: &str) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let deleted = conn.execute(
            "DELETE FROM obsidian_sync_state WHERE vault_name = ?1",
            params![vault_name],
        )?;
        Ok(deleted)
    }

    /// Derive `last_full_sync_at` as the MIN of all `last_synced_at` for the vault,
    /// or 0 if no entries exist.
    pub fn get_last_full_sync(&self, vault_name: &str) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let deleted = conn.execute(
            "DELETE FROM obsidian_write_queue WHERE id = ?1",
            params![id],
        )?;
        Ok(deleted > 0)
    }

    /// Clear all queued writes for a vault. Returns count deleted.
    pub fn clear_obsidian_queue(&self, vault_name: &str) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
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
        files: &HashMap<String, crate::agent::tools::obsidian::cache::CachedFileMeta>,
    ) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
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
    pub fn replace_obsidian_queue(
        &self,
        vault_name: &str,
        queue: &[crate::agent::tools::obsidian::cache::QueuedWrite],
    ) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM obsidian_write_queue WHERE vault_name = ?1",
            params![vault_name],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::super::MemoryDB;

    #[test]
    fn test_upsert_and_get_obsidian_sync() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.upsert_obsidian_sync("vault1", "notes/hello.md", "abc123", 1000, 42)
            .unwrap();

        let row = db
            .get_obsidian_sync("vault1", "notes/hello.md")
            .unwrap()
            .unwrap();
        assert_eq!(row.content_hash, "abc123");
        assert_eq!(row.last_synced_at, 1000);
        assert_eq!(row.size, 42);
    }

    #[test]
    fn test_upsert_obsidian_sync_replaces() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.upsert_obsidian_sync("vault1", "a.md", "old", 100, 10)
            .unwrap();
        db.upsert_obsidian_sync("vault1", "a.md", "new", 200, 20)
            .unwrap();

        let row = db.get_obsidian_sync("vault1", "a.md").unwrap().unwrap();
        assert_eq!(row.content_hash, "new");
        assert_eq!(row.last_synced_at, 200);
        assert_eq!(row.size, 20);
    }

    #[test]
    fn test_get_obsidian_sync_not_found() {
        let db = MemoryDB::new(":memory:").unwrap();
        assert!(
            db.get_obsidian_sync("vault1", "missing.md")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn test_list_obsidian_sync() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.upsert_obsidian_sync("vault1", "a.md", "h1", 100, 10)
            .unwrap();
        db.upsert_obsidian_sync("vault1", "b.md", "h2", 200, 20)
            .unwrap();
        db.upsert_obsidian_sync("vault2", "c.md", "h3", 300, 30)
            .unwrap();

        let map = db.list_obsidian_sync("vault1").unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map["a.md"].content_hash, "h1");
        assert_eq!(map["b.md"].content_hash, "h2");
    }

    #[test]
    fn test_remove_obsidian_sync() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.upsert_obsidian_sync("vault1", "a.md", "h1", 100, 10)
            .unwrap();
        assert!(db.remove_obsidian_sync("vault1", "a.md").unwrap());
        assert!(!db.remove_obsidian_sync("vault1", "a.md").unwrap());
        assert!(db.get_obsidian_sync("vault1", "a.md").unwrap().is_none());
    }

    #[test]
    fn test_clear_obsidian_sync() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.upsert_obsidian_sync("vault1", "a.md", "h1", 100, 10)
            .unwrap();
        db.upsert_obsidian_sync("vault1", "b.md", "h2", 200, 20)
            .unwrap();
        db.upsert_obsidian_sync("vault2", "c.md", "h3", 300, 30)
            .unwrap();

        assert_eq!(db.clear_obsidian_sync("vault1").unwrap(), 2);
        assert!(db.list_obsidian_sync("vault1").unwrap().is_empty());
        assert_eq!(db.list_obsidian_sync("vault2").unwrap().len(), 1);
    }

    #[test]
    fn test_get_last_full_sync() {
        let db = MemoryDB::new(":memory:").unwrap();

        assert_eq!(db.get_last_full_sync("vault1").unwrap(), 0);

        db.upsert_obsidian_sync("vault1", "a.md", "h1", 100, 10)
            .unwrap();
        db.upsert_obsidian_sync("vault1", "b.md", "h2", 200, 20)
            .unwrap();

        assert_eq!(db.get_last_full_sync("vault1").unwrap(), 100);
    }

    #[test]
    fn test_add_and_list_obsidian_queue() {
        let db = MemoryDB::new(":memory:").unwrap();

        let id1 = db
            .add_obsidian_queue("vault1", "a.md", "content1", "write", 1000, Some("hash1"))
            .unwrap();
        let id2 = db
            .add_obsidian_queue("vault1", "b.md", "content2", "append", 2000, None)
            .unwrap();

        assert!(id1 > 0);
        assert!(id2 > id1);

        let queue = db.list_obsidian_queue("vault1").unwrap();
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0].path, "a.md");
        assert_eq!(queue[0].content, "content1");
        assert_eq!(queue[0].operation, "write");
        assert_eq!(queue[0].queued_at, 1000);
        assert_eq!(queue[0].pre_write_hash, Some("hash1".to_string()));
        assert_eq!(queue[1].path, "b.md");
        assert!(queue[1].pre_write_hash.is_none());
    }

    #[test]
    fn test_remove_obsidian_queue() {
        let db = MemoryDB::new(":memory:").unwrap();

        let id = db
            .add_obsidian_queue("vault1", "a.md", "content", "write", 1000, None)
            .unwrap();
        assert!(db.remove_obsidian_queue(id).unwrap());
        assert!(!db.remove_obsidian_queue(id).unwrap());
        assert!(db.list_obsidian_queue("vault1").unwrap().is_empty());
    }

    #[test]
    fn test_clear_obsidian_queue() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.add_obsidian_queue("vault1", "a.md", "c1", "write", 1000, None)
            .unwrap();
        db.add_obsidian_queue("vault1", "b.md", "c2", "write", 2000, None)
            .unwrap();
        db.add_obsidian_queue("vault2", "c.md", "c3", "write", 3000, None)
            .unwrap();

        assert_eq!(db.clear_obsidian_queue("vault1").unwrap(), 2);
        assert!(db.list_obsidian_queue("vault1").unwrap().is_empty());
        assert_eq!(db.list_obsidian_queue("vault2").unwrap().len(), 1);
    }

    #[test]
    fn test_count_obsidian_queue() {
        let db = MemoryDB::new(":memory:").unwrap();

        assert_eq!(db.count_obsidian_queue("vault1").unwrap(), 0);

        db.add_obsidian_queue("vault1", "a.md", "c1", "write", 1000, None)
            .unwrap();
        db.add_obsidian_queue("vault1", "b.md", "c2", "write", 2000, None)
            .unwrap();

        assert_eq!(db.count_obsidian_queue("vault1").unwrap(), 2);
    }

    #[test]
    fn test_queue_vault_isolation() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.add_obsidian_queue("vault1", "a.md", "c1", "write", 1000, None)
            .unwrap();
        db.add_obsidian_queue("vault2", "b.md", "c2", "write", 2000, None)
            .unwrap();

        assert_eq!(db.count_obsidian_queue("vault1").unwrap(), 1);
        assert_eq!(db.count_obsidian_queue("vault2").unwrap(), 1);
    }
}
