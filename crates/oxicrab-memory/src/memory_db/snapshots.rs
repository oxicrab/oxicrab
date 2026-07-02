//! Point-in-time snapshots of durable memory, with restore.
//!
//! Memory in oxicrab is SQLite-backed and continuously mutated (fact
//! extraction, quality gates, dedup). That makes an accidental bad write —
//! or an agent-inferred fact that shouldn't have landed — hard to undo.
//!
//! A snapshot captures every `memory_entries` row as a versioned JSON
//! payload plus a content hash. `restore_snapshot` swaps the live table
//! back to that payload **inside a single transaction** (all-or-nothing:
//! a failed restore never leaves memory half-empty) and auto-captures a
//! `pre-restore` snapshot first, so restore is itself reversible.

use super::MemoryDB;
use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Payload schema version. Bump when the serialized row shape changes so
/// old snapshots are rejected clearly instead of restoring garbage.
const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// One serialized `memory_entries` row. Embeddings are intentionally NOT
/// captured — they are derived and get rebuilt by the backfill pass after
/// restore, so snapshots stay small and model-agnostic.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SnapshotEntry {
    source_key: String,
    content: String,
    content_hash: String,
    created_at: String,
}

/// Metadata for one stored snapshot (no payload).
#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub id: i64,
    pub label: String,
    pub entry_count: usize,
    pub content_sha256: String,
    pub created_at_ms: i64,
}

/// Outcome of a restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    /// Entries present after the restore.
    pub restored_entries: usize,
    /// Id of the auto-captured pre-restore snapshot (so the user can undo).
    pub pre_restore_snapshot_id: i64,
}

impl MemoryDB {
    /// Capture the current durable memory as a labeled snapshot. Returns
    /// the new snapshot id.
    pub fn snapshot_memory(&self, label: &str) -> Result<i64> {
        let conn = self.lock_conn()?;
        Self::snapshot_memory_inner(&conn, label)
    }

    /// Snapshot capture against an existing connection (so restore can
    /// capture the pre-restore snapshot inside its own transaction).
    fn snapshot_memory_inner(conn: &Connection, label: &str) -> Result<i64> {
        let entries = Self::read_all_entries(conn)?;
        let payload = serde_json::to_string(&entries)
            .context("failed to serialize memory snapshot payload")?;
        let mut hasher = Sha256::new();
        hasher.update(payload.as_bytes());
        let content_sha256 = hex::encode(hasher.finalize());
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO memory_snapshots
                (label, schema_version, entry_count, content_sha256, payload, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                label,
                SNAPSHOT_SCHEMA_VERSION,
                entries.len() as i64,
                content_sha256,
                payload,
                now,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Read every `memory_entries` row in a stable order.
    fn read_all_entries(conn: &Connection) -> Result<Vec<SnapshotEntry>> {
        let mut stmt = conn.prepare(
            "SELECT source_key, content, content_hash, created_at
               FROM memory_entries ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SnapshotEntry {
                source_key: r.get(0)?,
                content: r.get(1)?,
                content_hash: r.get(2)?,
                created_at: r.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// List stored snapshots, newest first.
    pub fn list_snapshots(&self, limit: usize) -> Result<Vec<SnapshotInfo>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, label, entry_count, content_sha256, created_at_ms
               FROM memory_snapshots ORDER BY created_at_ms DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| {
            Ok(SnapshotInfo {
                id: r.get(0)?,
                label: r.get(1)?,
                entry_count: r.get::<_, i64>(2)? as usize,
                content_sha256: r.get(3)?,
                created_at_ms: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Delete a snapshot by id. Returns true if a row was removed.
    pub fn delete_snapshot(&self, id: i64) -> Result<bool> {
        let conn = self.lock_conn()?;
        let n = conn.execute("DELETE FROM memory_snapshots WHERE id = ?1", [id])?;
        Ok(n > 0)
    }

    /// Restore durable memory to a stored snapshot.
    ///
    /// Auto-captures a `pre-restore` snapshot first (so the restore is
    /// reversible), then — inside a single transaction — clears
    /// `memory_entries` (and cascades embeddings) and reinserts the
    /// snapshot rows verbatim, preserving original `created_at`. Rebuilds
    /// `memory_sources` from the restored rows and invalidates the
    /// embedding cache so the backfill pass re-embeds.
    pub fn restore_snapshot(&self, id: i64) -> Result<RestoreOutcome> {
        let mut conn = self.lock_conn()?;

        // Load + validate the target payload before mutating anything.
        let (schema_version, payload): (u32, String) = conn
            .query_row(
                "SELECT schema_version, payload FROM memory_snapshots WHERE id = ?1",
                [id],
                |r| Ok((r.get::<_, i64>(0)? as u32, r.get(1)?)),
            )
            .with_context(|| format!("snapshot #{id} not found"))?;
        if schema_version != SNAPSHOT_SCHEMA_VERSION {
            anyhow::bail!(
                "snapshot #{id} has schema version {schema_version}, but this build expects \
                 {SNAPSHOT_SCHEMA_VERSION}; refusing to restore an incompatible payload"
            );
        }
        let entries: Vec<SnapshotEntry> = serde_json::from_str(&payload)
            .with_context(|| format!("snapshot #{id} payload is corrupt"))?;

        // Reversibility: capture current state before overwriting it.
        let pre_restore_snapshot_id = Self::snapshot_memory_inner(&conn, "pre-restore")?;

        let now = chrono::Utc::now().to_rfc3339();
        let tx = conn.transaction()?;
        // Embeddings cascade via ON DELETE CASCADE on memory_embeddings.
        tx.execute("DELETE FROM memory_entries", [])?;
        tx.execute("DELETE FROM memory_sources", [])?;
        for e in &entries {
            tx.execute(
                "INSERT OR IGNORE INTO memory_entries
                    (source_key, content, content_hash, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![e.source_key, e.content, e.content_hash, e.created_at],
            )?;
            tx.execute(
                "INSERT INTO memory_sources (source_key, mtime_ns, updated_at)
                 VALUES (?1, 0, ?2)
                 ON CONFLICT(source_key) DO UPDATE SET updated_at = excluded.updated_at",
                params![e.source_key, now],
            )?;
        }
        tx.commit()?;

        // Derived embeddings are now stale/absent; let the backfill pass rebuild.
        self.invalidate_embedding_cache();

        let restored_entries = Self::read_all_entries(&conn)?.len();
        Ok(RestoreOutcome {
            restored_entries,
            pre_restore_snapshot_id,
        })
    }
}

#[cfg(test)]
mod tests;
