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
mod oauth;
mod pairing;
mod search;
mod stats;
mod workspace;

pub use cost::TokenSummaryRow;
pub use dlq::DlqEntry;
pub use oauth::OAuthTokenRow;
pub use search::MemoryHit;
pub use stats::{
    ComplexityEvent, ComplexityForceCount, ComplexityStats, ComplexityTierStats, IntentEvent,
    IntentStats, SearchStats,
};
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

        // Column named mtime_ns for backwards compat (actually stores milliseconds)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS memory_sources (
                source_key TEXT PRIMARY KEY,
                mtime_ns INTEGER NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS memory_entries (
                id INTEGER PRIMARY KEY,
                source_key TEXT NOT NULL,
                content TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE (source_key, content_hash)
            )",
            [],
        )?;

        // Create embeddings table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS memory_embeddings (
                entry_id INTEGER PRIMARY KEY REFERENCES memory_entries(id) ON DELETE CASCADE,
                embedding BLOB NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS memory_access_log (
                id INTEGER PRIMARY KEY,
                query TEXT NOT NULL,
                search_type TEXT NOT NULL,
                result_count INTEGER NOT NULL,
                top_score REAL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS memory_search_hits (
                id INTEGER PRIMARY KEY,
                access_log_id INTEGER NOT NULL REFERENCES memory_access_log(id),
                source_key TEXT NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_search_hits_source ON memory_search_hits(source_key)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_search_hits_log_id ON memory_search_hits(access_log_id)",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS llm_cost_log (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL DEFAULT (datetime('now')),
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cost_cents REAL NOT NULL,
                caller TEXT NOT NULL DEFAULT 'main'
            )",
            [],
        )?;

        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_cost_log_date ON llm_cost_log(timestamp);
             CREATE INDEX IF NOT EXISTS idx_cost_log_model ON llm_cost_log(model);",
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS scheduled_task_dlq (
                id INTEGER PRIMARY KEY,
                job_id TEXT NOT NULL,
                job_name TEXT NOT NULL,
                payload TEXT NOT NULL,
                error_message TEXT NOT NULL,
                failed_at TEXT NOT NULL DEFAULT (datetime('now')),
                retry_count INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'pending_retry'
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS intent_metrics (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL DEFAULT (datetime('now')),
                event_type TEXT NOT NULL,
                intent_method TEXT,
                semantic_score REAL,
                detection_layer TEXT,
                message_preview TEXT
            )",
            [],
        )?;

        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_intent_metrics_date ON intent_metrics(timestamp);
             CREATE INDEX IF NOT EXISTS idx_intent_metrics_type ON intent_metrics(event_type);",
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS workspace_files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                category TEXT NOT NULL,
                original_name TEXT,
                size_bytes INTEGER NOT NULL,
                source_tool TEXT,
                tags TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                accessed_at TEXT,
                session_key TEXT
            )",
            [],
        )?;

        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_workspace_files_category ON workspace_files(category);
             CREATE INDEX IF NOT EXISTS idx_workspace_files_created ON workspace_files(created_at);",
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS complexity_routing_log (
                id INTEGER PRIMARY KEY,
                request_id TEXT NOT NULL,
                timestamp TEXT NOT NULL DEFAULT (datetime('now')),
                composite_score REAL NOT NULL,
                resolved_tier TEXT NOT NULL,
                resolved_model TEXT,
                forced TEXT,
                channel TEXT,
                message_preview TEXT
            )",
            [],
        )?;

        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_complexity_log_ts ON complexity_routing_log(timestamp);
             CREATE INDEX IF NOT EXISTS idx_complexity_log_req ON complexity_routing_log(request_id);",
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS cron_jobs (
                id               TEXT PRIMARY KEY,
                name             TEXT NOT NULL,
                enabled          INTEGER NOT NULL DEFAULT 1,
                schedule_type    TEXT NOT NULL,
                at_ms            INTEGER,
                every_ms         INTEGER,
                cron_expr        TEXT,
                cron_tz          TEXT,
                event_pattern    TEXT,
                event_channel    TEXT,
                payload_kind     TEXT NOT NULL DEFAULT 'agent_turn',
                payload_message  TEXT NOT NULL DEFAULT '',
                agent_echo       INTEGER NOT NULL DEFAULT 1,
                next_run_at_ms   INTEGER,
                last_run_at_ms   INTEGER,
                last_status      TEXT,
                last_error       TEXT,
                run_count        INTEGER NOT NULL DEFAULT 0,
                last_fired_at_ms INTEGER,
                created_at_ms    INTEGER NOT NULL,
                updated_at_ms    INTEGER NOT NULL,
                delete_after_run INTEGER NOT NULL DEFAULT 0,
                expires_at_ms    INTEGER,
                max_runs         INTEGER,
                cooldown_secs    INTEGER,
                max_concurrent   INTEGER
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_cron_jobs_enabled_next
             ON cron_jobs(enabled, next_run_at_ms)",
            [],
        )?;

        // --- Pairing tables ---

        conn.execute(
            "CREATE TABLE IF NOT EXISTS pairing_allowlist (
                channel    TEXT NOT NULL,
                sender_id  TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (channel, sender_id)
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS pairing_pending (
                channel    TEXT NOT NULL,
                sender_id  TEXT NOT NULL,
                code       TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS pairing_failed_attempts (
                client_id    TEXT NOT NULL,
                attempted_at INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pairing_failed_client
             ON pairing_failed_attempts(client_id)",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS cron_job_targets (
                job_id   TEXT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
                channel  TEXT NOT NULL,
                target   TEXT NOT NULL,
                PRIMARY KEY (job_id, channel, target)
            )",
            [],
        )?;

        // Add request_id to existing tables (idempotent: ignore if column already exists)
        for table in &["llm_cost_log", "intent_metrics", "memory_access_log"] {
            let _ = conn.execute(
                &format!("ALTER TABLE {table} ADD COLUMN request_id TEXT"),
                [],
            );
        }

        conn.execute(
            "CREATE TABLE IF NOT EXISTS sessions (
                key TEXT PRIMARY KEY,
                data TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS oauth_tokens (
                provider      TEXT PRIMARY KEY,
                access_token  TEXT NOT NULL,
                refresh_token TEXT,
                expires_at    INTEGER NOT NULL,
                extra_json    TEXT,
                updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
            )",
            [],
        )?;

        // Try to create FTS5 virtual table
        if conn
            .execute(
                "CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts
            USING fts5(
                content,
                source_key,
                content='memory_entries',
                content_rowid='id'
            )",
                [],
            )
            .is_ok()
        {
            // Create triggers
            conn.execute(
                "CREATE TRIGGER IF NOT EXISTS mem_ai AFTER INSERT ON memory_entries BEGIN
                    INSERT INTO memory_fts(rowid, content, source_key)
                    VALUES (new.id, new.content, new.source_key);
                END",
                [],
            )?;

            conn.execute(
                "CREATE TRIGGER IF NOT EXISTS mem_ad AFTER DELETE ON memory_entries BEGIN
                    INSERT INTO memory_fts(memory_fts, rowid, content, source_key)
                    VALUES ('delete', old.id, old.content, old.source_key);
                END",
                [],
            )?;

            conn.execute(
                "CREATE TRIGGER IF NOT EXISTS mem_au AFTER UPDATE ON memory_entries BEGIN
                    INSERT INTO memory_fts(memory_fts, rowid, content, source_key)
                    VALUES ('delete', old.id, old.content, old.source_key);
                    INSERT INTO memory_fts(rowid, content, source_key)
                    VALUES (new.id, new.content, new.source_key);
                END",
                [],
            )?;

            self.has_fts = true;
        } else {
            self.has_fts = false;
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
