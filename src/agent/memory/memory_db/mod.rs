use anyhow::{Context, Result};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::path::Path;
use tracing::warn;

mod cost;
mod cron;
mod dlq;
mod embeddings;
mod indexing;
mod migrations;
mod oauth;
mod obsidian;
mod pairing;
mod search;
mod stats;
mod subagent_log;
mod workspace;

pub use cost::TokenSummaryRow;
pub use dlq::DlqEntry;
pub use oauth::OAuthTokenRow;
pub use pairing::DbPendingRequest;
pub use search::MemoryHit;
pub use stats::SearchDetails;
pub use stats::{
    ComplexityEvent, ComplexityForceCount, ComplexityStats, ComplexityTierStats, IntentEvent,
    IntentStats, SearchStats,
};
pub use subagent_log::SubagentLogEntry;
pub use workspace::WorkspaceFileEntry;

use embeddings::CachedEmbedding;
#[cfg(test)]
use search::MAX_FTS_TERMS;
#[cfg(test)]
use search::fts_query;

/// Compute a recency decay multiplier for a BM25 score.
///
/// Uses exponential decay: `0.5 ^ (age_days / half_life_days)`.
/// Returns 1.0 for fresh entries, 0.5 at one half-life, 0.25 at two, etc.
/// A `half_life_days` of 0 disables decay (returns 1.0).
pub fn recency_decay(age_days: f64, half_life_days: u32) -> f32 {
    if half_life_days == 0 || age_days <= 0.0 {
        return 1.0;
    }
    (0.5_f64.powf(age_days / f64::from(half_life_days))) as f32
}

pub struct MemoryDB {
    pub(crate) conn: std::sync::Mutex<Connection>,
    db_path: String,
    has_fts: bool,
    /// Lazily populated embedding cache. Set to `None` to invalidate
    /// (e.g. after `store_embedding`). Avoids re-reading and deserializing
    /// all embeddings from `SQLite` on every `hybrid_search` call.
    embedding_cache: std::sync::Mutex<Option<Vec<CachedEmbedding>>>,
}

impl Clone for MemoryDB {
    fn clone(&self) -> Self {
        // Re-open a connection for clones (rare, needed for spawn_blocking patterns).
        // Panics on failure because callers depend on the clone being connected
        // to the same database file — an in-memory fallback would silently lose data.
        let conn = Connection::open(&self.db_path).unwrap_or_else(|e| {
            panic!(
                "failed to re-open memory DB at {} for clone: {}",
                self.db_path, e
            )
        });
        if let Err(e) = conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=3000;
             PRAGMA foreign_keys=ON;",
        ) {
            warn!(
                "failed to set PRAGMAs on cloned DB connection: {} \
                 (WAL mode, busy timeout, and foreign keys may not be active)",
                e
            );
        }
        Self {
            conn: std::sync::Mutex::new(conn),
            db_path: self.db_path.clone(),
            has_fts: self.has_fts,
            embedding_cache: std::sync::Mutex::new(None),
        }
    }
}

impl MemoryDB {
    pub fn new(db_path: impl AsRef<Path>) -> Result<Self> {
        let db_path = db_path.as_ref();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create database parent directory: {}",
                    parent.display()
                )
            })?;
        }

        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open database at: {}", db_path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=3000;
             PRAGMA foreign_keys=ON;",
        )?;

        let mut db = Self {
            conn: std::sync::Mutex::new(conn),
            db_path: db_path.to_string_lossy().to_string(),
            has_fts: false,
            embedding_cache: std::sync::Mutex::new(None),
        };

        db.ensure_schema().with_context(|| {
            format!(
                "Failed to initialize database schema at: {}",
                db_path.display()
            )
        })?;
        Ok(db)
    }

    fn ensure_schema(&mut self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;

        migrations::apply_migrations(&conn)?;
        self.has_fts = migrations::ensure_fts_objects(&conn)?;
        if !self.has_fts {
            warn!("FTS5 not available; search will use LIKE fallback (degraded quality)");
        }

        Ok(())
    }
}

// --- Session storage ---

impl MemoryDB {
    /// Load a session by key. Returns `None` if not found.
    pub fn load_session(&self, key: &str) -> Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare("SELECT data FROM sessions WHERE key = ?1")?;
        let mut rows = stmt.query(rusqlite::params![key])?;
        if let Some(row) = rows.next()? {
            let data: String = row.get(0)?;
            Ok(Some(data))
        } else {
            Ok(None)
        }
    }

    /// Save a session (insert or replace).
    pub fn save_session(&self, key: &str, data: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        conn.execute(
            "INSERT OR REPLACE INTO sessions (key, data, updated_at) VALUES (?1, ?2, datetime('now'))",
            rusqlite::params![key, data],
        )?;
        Ok(())
    }

    /// Delete sessions not updated within `ttl_days`. Returns count deleted.
    /// A TTL of 0 deletes all sessions.
    pub fn cleanup_sessions(&self, ttl_days: u32) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let deleted = if ttl_days == 0 {
            conn.execute("DELETE FROM sessions", [])?
        } else {
            conn.execute(
                "DELETE FROM sessions WHERE updated_at < datetime('now', ?1)",
                rusqlite::params![format!("-{ttl_days} days")],
            )?
        };
        Ok(deleted)
    }
}

fn hash_text(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests;
