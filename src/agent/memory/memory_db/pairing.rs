use super::MemoryDB;
use anyhow::Result;
use rusqlite::params;

/// A pending pairing request returned from DB queries.
#[derive(Debug, Clone)]
pub struct DbPendingRequest {
    pub channel: String,
    pub sender_id: String,
    pub code: String,
    pub created_at: u64,
}

impl MemoryDB {
    /// Add a sender to the pairing allowlist. Returns `true` if newly inserted.
    pub fn add_paired_sender(&self, channel: &str, sender_id: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO pairing_allowlist (channel, sender_id) VALUES (?1, ?2)",
            params![channel, sender_id],
        )?;
        Ok(inserted > 0)
    }

    /// Remove a sender from the pairing allowlist. Returns `true` if removed.
    pub fn remove_paired_sender(&self, channel: &str, sender_id: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let deleted = conn.execute(
            "DELETE FROM pairing_allowlist WHERE channel = ?1 AND sender_id = ?2",
            params![channel, sender_id],
        )?;
        Ok(deleted > 0)
    }

    /// Check if a sender is paired for a channel.
    pub fn is_sender_paired(&self, channel: &str, sender_id: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM pairing_allowlist WHERE channel = ?1 AND sender_id = ?2 LIMIT 1",
                params![channel, sender_id],
                |_| Ok(true),
            )
            .unwrap_or(false);
        Ok(exists)
    }

    /// List all paired senders for a channel.
    pub fn list_paired_senders(&self, channel: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt =
            conn.prepare("SELECT sender_id FROM pairing_allowlist WHERE channel = ?1")?;
        let rows = stmt
            .query_map(params![channel], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Count total paired senders across all channels.
    pub fn count_paired_senders(&self) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM pairing_allowlist", [], |row| {
            row.get(0)
        })?;
        Ok(count as usize)
    }

    /// List all paired channels and their senders.
    pub fn list_all_paired_channels(&self) -> Result<Vec<(String, Vec<String>)>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT channel, sender_id FROM pairing_allowlist ORDER BY channel, sender_id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut result: Vec<(String, Vec<String>)> = Vec::new();
        for (channel, sender_id) in rows {
            if let Some(last) = result.last_mut()
                && last.0 == channel
            {
                last.1.push(sender_id);
                continue;
            }
            result.push((channel, vec![sender_id]));
        }
        Ok(result)
    }

    /// Add a pending pairing request.
    pub fn add_pending_request(
        &self,
        channel: &str,
        sender_id: &str,
        code: &str,
        created_at: u64,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT INTO pairing_pending (channel, sender_id, code, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![channel, sender_id, code, created_at as i64],
        )?;
        Ok(())
    }

    /// Get all non-expired pending requests.
    /// Returns all requests where `now - created_at < ttl_secs`.
    /// The caller does constant-time code comparison in Rust.
    pub fn get_all_pending(&self, ttl_secs: u64) -> Result<Vec<DbPendingRequest>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cutoff = now.saturating_sub(ttl_secs) as i64;

        let mut stmt = conn.prepare(
            "SELECT channel, sender_id, code, created_at FROM pairing_pending
             WHERE created_at > ?1",
        )?;
        let rows = stmt
            .query_map(params![cutoff], |row| {
                Ok(DbPendingRequest {
                    channel: row.get(0)?,
                    sender_id: row.get(1)?,
                    code: row.get(2)?,
                    created_at: row.get::<_, i64>(3)? as u64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Get a pending request for a specific sender on a channel (non-expired).
    pub fn get_pending_for_sender(
        &self,
        channel: &str,
        sender_id: &str,
        ttl_secs: u64,
    ) -> Result<Option<DbPendingRequest>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cutoff = now.saturating_sub(ttl_secs) as i64;

        let mut stmt = conn.prepare(
            "SELECT channel, sender_id, code, created_at FROM pairing_pending
             WHERE channel = ?1 AND sender_id = ?2 AND created_at > ?3",
        )?;
        let mut rows = stmt.query(params![channel, sender_id, cutoff])?;
        if let Some(row) = rows.next()? {
            Ok(Some(DbPendingRequest {
                channel: row.get(0)?,
                sender_id: row.get(1)?,
                code: row.get(2)?,
                created_at: row.get::<_, i64>(3)? as u64,
            }))
        } else {
            Ok(None)
        }
    }

    /// Count non-expired pending requests for a channel.
    pub fn count_pending_for_channel(&self, channel: &str, ttl_secs: u64) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cutoff = now.saturating_sub(ttl_secs) as i64;

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pairing_pending WHERE channel = ?1 AND created_at > ?2",
            params![channel, cutoff],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Remove a pending request by code. Returns `true` if removed.
    pub fn remove_pending(&self, code: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let deleted = conn.execute("DELETE FROM pairing_pending WHERE code = ?1", params![code])?;
        Ok(deleted > 0)
    }

    /// Clean up expired pending requests. Returns count removed.
    pub fn cleanup_expired_pending(&self, ttl_secs: u64) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cutoff = now.saturating_sub(ttl_secs) as i64;

        let deleted = conn.execute(
            "DELETE FROM pairing_pending WHERE created_at <= ?1",
            params![cutoff],
        )?;
        Ok(deleted)
    }

    /// Record a failed approval attempt.
    pub fn record_failed_attempt(&self, client_id: &str, timestamp: u64) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT INTO pairing_failed_attempts (client_id, attempted_at) VALUES (?1, ?2)",
            params![client_id, timestamp as i64],
        )?;
        Ok(())
    }

    /// Count recent failed attempts for a client within a time window.
    pub fn count_recent_failed_attempts(&self, client_id: &str, window_secs: u64) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cutoff = now.saturating_sub(window_secs) as i64;

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pairing_failed_attempts
             WHERE client_id = ?1 AND attempted_at > ?2",
            params![client_id, cutoff],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Clean up old failed attempts outside the window. Returns count removed.
    pub fn cleanup_old_failed_attempts(&self, window_secs: u64) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cutoff = now.saturating_sub(window_secs) as i64;

        let deleted = conn.execute(
            "DELETE FROM pairing_failed_attempts WHERE attempted_at <= ?1",
            params![cutoff],
        )?;
        Ok(deleted)
    }

    /// Evict the oldest lockout client if we exceed `max_clients`.
    pub fn evict_oldest_lockout_client(&self, max_clients: usize) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let distinct_count: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT client_id) FROM pairing_failed_attempts",
            [],
            |row| row.get(0),
        )?;

        if (distinct_count as usize) > max_clients {
            // Find the client whose most-recent attempt is oldest
            conn.execute(
                "DELETE FROM pairing_failed_attempts WHERE client_id = (
                    SELECT client_id FROM pairing_failed_attempts
                    GROUP BY client_id
                    ORDER BY MAX(attempted_at) ASC
                    LIMIT 1
                )",
                [],
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::MemoryDB;

    #[test]
    fn test_add_and_check_paired_sender() {
        let db = MemoryDB::new(":memory:").unwrap();

        assert!(!db.is_sender_paired("telegram", "user1").unwrap());
        assert!(db.add_paired_sender("telegram", "user1").unwrap());
        assert!(db.is_sender_paired("telegram", "user1").unwrap());

        // Duplicate insert returns false
        assert!(!db.add_paired_sender("telegram", "user1").unwrap());
    }

    #[test]
    fn test_remove_paired_sender() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.add_paired_sender("telegram", "user1").unwrap();
        assert!(db.remove_paired_sender("telegram", "user1").unwrap());
        assert!(!db.is_sender_paired("telegram", "user1").unwrap());

        // Removing non-existent returns false
        assert!(!db.remove_paired_sender("telegram", "user1").unwrap());
    }

    #[test]
    fn test_list_paired_senders() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.add_paired_sender("telegram", "alice").unwrap();
        db.add_paired_sender("telegram", "bob").unwrap();
        db.add_paired_sender("discord", "charlie").unwrap();

        let tg = db.list_paired_senders("telegram").unwrap();
        assert_eq!(tg.len(), 2);
        assert!(tg.contains(&"alice".to_string()));
        assert!(tg.contains(&"bob".to_string()));

        let dc = db.list_paired_senders("discord").unwrap();
        assert_eq!(dc.len(), 1);
    }

    #[test]
    fn test_count_paired_senders() {
        let db = MemoryDB::new(":memory:").unwrap();

        assert_eq!(db.count_paired_senders().unwrap(), 0);
        db.add_paired_sender("telegram", "a").unwrap();
        db.add_paired_sender("discord", "b").unwrap();
        assert_eq!(db.count_paired_senders().unwrap(), 2);
    }

    #[test]
    fn test_list_all_paired_channels() {
        let db = MemoryDB::new(":memory:").unwrap();

        db.add_paired_sender("discord", "user1").unwrap();
        db.add_paired_sender("discord", "user2").unwrap();
        db.add_paired_sender("telegram", "user3").unwrap();

        let all = db.list_all_paired_channels().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].0, "discord");
        assert_eq!(all[0].1.len(), 2);
        assert_eq!(all[1].0, "telegram");
        assert_eq!(all[1].1.len(), 1);
    }

    #[test]
    fn test_pending_request_lifecycle() {
        let db = MemoryDB::new(":memory:").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        db.add_pending_request("telegram", "user1", "ABCD1234", now)
            .unwrap();

        let pending = db.get_pending_for_sender("telegram", "user1", 900).unwrap();
        assert!(pending.is_some());
        let p = pending.unwrap();
        assert_eq!(p.code, "ABCD1234");

        assert_eq!(db.count_pending_for_channel("telegram", 900).unwrap(), 1);

        assert!(db.remove_pending("ABCD1234").unwrap());
        assert!(!db.remove_pending("ABCD1234").unwrap());

        assert_eq!(db.count_pending_for_channel("telegram", 900).unwrap(), 0);
    }

    #[test]
    fn test_get_all_pending() {
        let db = MemoryDB::new(":memory:").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        db.add_pending_request("telegram", "u1", "CODE1111", now)
            .unwrap();
        db.add_pending_request("discord", "u2", "CODE2222", now)
            .unwrap();
        // Expired request
        db.add_pending_request("slack", "u3", "CODE3333", now.saturating_sub(1000))
            .unwrap();

        let all = db.get_all_pending(900).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_cleanup_expired_pending() {
        let db = MemoryDB::new(":memory:").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        db.add_pending_request("telegram", "u1", "FRESH111", now)
            .unwrap();
        db.add_pending_request("telegram", "u2", "OLD22222", now.saturating_sub(1000))
            .unwrap();

        let cleaned = db.cleanup_expired_pending(900).unwrap();
        assert_eq!(cleaned, 1);

        let all = db.get_all_pending(900).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].code, "FRESH111");
    }

    #[test]
    fn test_failed_attempts() {
        let db = MemoryDB::new(":memory:").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        db.record_failed_attempt("admin1", now).unwrap();
        db.record_failed_attempt("admin1", now).unwrap();
        db.record_failed_attempt("admin2", now).unwrap();

        assert_eq!(db.count_recent_failed_attempts("admin1", 300).unwrap(), 2);
        assert_eq!(db.count_recent_failed_attempts("admin2", 300).unwrap(), 1);
        assert_eq!(db.count_recent_failed_attempts("admin3", 300).unwrap(), 0);
    }

    #[test]
    fn test_cleanup_old_failed_attempts() {
        let db = MemoryDB::new(":memory:").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        db.record_failed_attempt("admin1", now).unwrap();
        db.record_failed_attempt("admin1", now.saturating_sub(600))
            .unwrap();

        let cleaned = db.cleanup_old_failed_attempts(300).unwrap();
        assert_eq!(cleaned, 1);
        assert_eq!(db.count_recent_failed_attempts("admin1", 300).unwrap(), 1);
    }

    #[test]
    fn test_evict_oldest_lockout_client() {
        let db = MemoryDB::new(":memory:").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Add 3 clients with different timestamps
        db.record_failed_attempt("oldest", now.saturating_sub(100))
            .unwrap();
        db.record_failed_attempt("middle", now.saturating_sub(50))
            .unwrap();
        db.record_failed_attempt("newest", now).unwrap();

        // Evict if more than 2 clients
        db.evict_oldest_lockout_client(2).unwrap();

        // "oldest" should be evicted
        assert_eq!(db.count_recent_failed_attempts("oldest", 300).unwrap(), 0);
        assert_eq!(db.count_recent_failed_attempts("middle", 300).unwrap(), 1);
        assert_eq!(db.count_recent_failed_attempts("newest", 300).unwrap(), 1);
    }
}
