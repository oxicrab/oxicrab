use crate::config::FusionStrategy;
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::Path;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct MemoryHit {
    pub source_key: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct CostSummaryRow {
    pub date: String,
    pub model: String,
    pub total_cents: f64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub call_count: i64,
}

#[derive(Debug, Clone)]
pub struct SearchStats {
    pub total_searches: u64,
    pub total_hits: u64,
    pub avg_results_per_search: f64,
}

#[derive(Debug, Clone)]
pub struct IntentStats {
    pub total_classified: u64,
    pub regex_action: u64,
    pub semantic_action: u64,
    pub not_action: u64,
    pub hallucinations_caught: u64,
    pub layer1_regex: u64,
    pub layer2_intent: u64,
    pub avg_semantic_score_action: f64,
    pub avg_semantic_score_non_action: f64,
}

#[derive(Debug, Clone)]
pub struct IntentEvent {
    pub timestamp: String,
    pub detection_layer: Option<String>,
    pub message_preview: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DlqEntry {
    pub id: i64,
    pub job_id: String,
    pub job_name: String,
    pub payload: String,
    pub error_message: String,
    pub failed_at: String,
    pub retry_count: i64,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceFileEntry {
    pub id: i64,
    pub path: String,
    pub category: String,
    pub original_name: Option<String>,
    pub size_bytes: i64,
    pub source_tool: Option<String>,
    pub tags: String,
    pub created_at: String,
    pub accessed_at: Option<String>,
    pub session_key: Option<String>,
}

/// Minimum size for a memory chunk (paragraphs shorter than this are skipped)
const MIN_CHUNK_SIZE: usize = 12;
/// Maximum size for a memory chunk (longer paragraphs are truncated)
const MAX_CHUNK_SIZE: usize = 1200;
/// Maximum number of unique terms used in FTS queries
const MAX_FTS_TERMS: usize = 16;

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

/// Cached deserialized embedding for in-memory vector search.
#[derive(Clone)]
struct CachedEmbedding {
    entry_id: i64,
    source_key: String,
    content: String,
    embedding: Vec<f32>,
}

pub struct MemoryDB {
    conn: std::sync::Mutex<Connection>,
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

        const {
            assert!(
                MIN_CHUNK_SIZE < MAX_CHUNK_SIZE,
                "MIN_CHUNK_SIZE must be less than MAX_CHUNK_SIZE"
            );
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
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;

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

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;

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

        // Wipe old entries
        conn.execute(
            "DELETE FROM memory_entries WHERE source_key = ?",
            [source_key],
        )?;

        for chunk in split_into_chunks(text) {
            let hash = hash_text(&chunk);
            conn.execute(
                "INSERT OR IGNORE INTO memory_entries
                    (source_key, content, content_hash, created_at)
                VALUES (?, ?, ?, ?)",
                params![source_key, chunk, hash, now],
            )?;
        }

        // Update source record
        conn.execute(
            "INSERT INTO memory_sources (source_key, mtime_ns, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(source_key)
            DO UPDATE SET mtime_ns = excluded.mtime_ns,
                          updated_at = excluded.updated_at",
            params![source_key, mtime_ms, now],
        )?;

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

        for entry in std::fs::read_dir(knowledge_dir)? {
            let entry = entry?;
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !matches!(ext.as_str(), "md" | "txt" | "html") {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let source_key = format!("knowledge:{}", name);
            if ext == "html" {
                self.index_html_file(&source_key, &path)?;
            } else {
                self.index_file(&source_key, &path)?;
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

    /// Store an embedding for a memory entry.
    ///
    /// Invalidates the in-memory embedding cache so the next `hybrid_search`
    /// picks up the new data.
    pub fn store_embedding(&self, entry_id: i64, embedding: &[u8]) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        conn.execute(
            "INSERT OR REPLACE INTO memory_embeddings (entry_id, embedding) VALUES (?, ?)",
            params![entry_id, embedding],
        )?;
        // Invalidate cached embeddings so hybrid_search reloads from DB
        if let Ok(mut cache) = self.embedding_cache.lock() {
            *cache = None;
        }
        Ok(())
    }

    /// Get all embeddings, optionally excluding certain source keys.
    /// Returns (`entry_id`, `source_key`, content, `embedding_blob`).
    #[allow(clippy::type_complexity)]
    pub fn get_all_embeddings(
        &self,
        exclude_sources: Option<&std::collections::HashSet<String>>,
    ) -> Result<Vec<(i64, String, String, Vec<u8>)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT me.id, me.content, me.source_key, emb.embedding
             FROM memory_embeddings emb
             JOIN memory_entries me ON emb.entry_id = me.id",
        )?;

        let default_set = std::collections::HashSet::new();
        let exclude = exclude_sources.unwrap_or(&default_set);

        let rows: Result<Vec<_>, _> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            })?
            .collect();

        Ok(rows
            .map_err(|e| anyhow::anyhow!("Failed to get embeddings: {}", e))?
            .into_iter()
            .filter(|(_, _, key, _)| !exclude.contains(key))
            .collect())
    }

    /// Get or populate the in-memory embedding cache.
    /// Returns cached deserialized embeddings, loading from DB on first call
    /// or after invalidation (e.g. after `store_embedding`).
    fn get_cached_embeddings(
        &self,
        exclude_sources: Option<&std::collections::HashSet<String>>,
    ) -> Result<Vec<CachedEmbedding>> {
        use crate::agent::memory::embeddings::deserialize_embedding;

        let default_set = std::collections::HashSet::new();
        let exclude = exclude_sources.unwrap_or(&default_set);

        // Check cache first
        if let Ok(cache) = self.embedding_cache.lock()
            && let Some(ref cached) = *cache
        {
            return Ok(cached
                .iter()
                .filter(|e| !exclude.contains(&e.source_key))
                .cloned()
                .collect());
        }

        // Cache miss — load from DB, deserialize, and cache
        let raw = self.get_all_embeddings(None)?;
        let mut entries = Vec::with_capacity(raw.len());
        for (entry_id, source_key, content, emb_bytes) in raw {
            match deserialize_embedding(&emb_bytes) {
                Ok(embedding) => entries.push(CachedEmbedding {
                    entry_id,
                    source_key,
                    content,
                    embedding,
                }),
                Err(e) => {
                    warn!("skipping corrupted embedding for entry {entry_id}: {e}");
                }
            }
        }

        // Store in cache (unfiltered so it can be reused with different excludes)
        if let Ok(mut cache) = self.embedding_cache.lock() {
            *cache = Some(entries.clone());
        }

        Ok(entries
            .into_iter()
            .filter(|e| !exclude.contains(&e.source_key))
            .collect())
    }

    /// Hybrid search combining FTS5 BM25 and vector cosine similarity.
    /// `keyword_weight` controls blending: 1.0 = keyword only, 0.0 = vector only.
    /// `fusion_strategy` selects the score combination method:
    /// - `WeightedScore`: linear blend of normalized scores
    /// - `Rrf`: reciprocal rank fusion (ignores raw scores, merges by rank)
    #[allow(clippy::too_many_arguments)]
    pub fn hybrid_search(
        &self,
        query_text: &str,
        query_embedding: &[f32],
        limit: usize,
        exclude_sources: Option<&std::collections::HashSet<String>>,
        keyword_weight: f32,
        fusion_strategy: FusionStrategy,
        rrf_k: u32,
        recency_half_life_days: u32,
    ) -> Result<Vec<MemoryHit>> {
        use crate::agent::memory::embeddings::cosine_similarity;

        if query_embedding.is_empty() {
            anyhow::bail!("query embedding is empty");
        }

        let default_set = std::collections::HashSet::new();
        let exclude = exclude_sources.unwrap_or(&default_set);

        // 1. Get FTS5 results with BM25 scores
        let mut fts_scores: std::collections::HashMap<i64, (f32, String, String)> =
            std::collections::HashMap::new();

        if keyword_weight > 0.0 {
            let query = fts_query(query_text);
            if !query.is_empty() && self.has_fts {
                let conn = self
                    .conn
                    .lock()
                    .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
                let mut stmt = conn.prepare(
                    "SELECT me.id, me.source_key, me.content, bm25(memory_fts) as score, me.created_at
                     FROM memory_fts
                     JOIN memory_entries me ON memory_fts.rowid = me.id
                     WHERE memory_fts MATCH ?
                     ORDER BY bm25(memory_fts)
                     LIMIT 100",
                )?;

                let now = Utc::now();
                let rows: Vec<_> = stmt
                    .query_map([&query], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, f64>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    })?
                    .filter_map(std::result::Result::ok)
                    .filter(|(_, key, _, _, _)| !exclude.contains(key))
                    .collect();

                // BM25 scores are negative (more negative = better match).
                // Normalize to 0..1 range, then apply recency decay.
                if !rows.is_empty() {
                    let min_score = rows
                        .iter()
                        .map(|(_, _, _, s, _)| *s)
                        .fold(f64::INFINITY, f64::min);
                    let max_score = rows
                        .iter()
                        .map(|(_, _, _, s, _)| *s)
                        .fold(f64::NEG_INFINITY, f64::max);
                    let range = max_score - min_score;

                    for (id, key, content, score, created_at) in rows {
                        let normalized = if range.abs() < 1e-10 {
                            1.0
                        } else {
                            // Invert: most negative (best) -> 1.0, least negative (worst) -> 0.0
                            ((max_score - score) / range) as f32
                        };
                        let age_days = chrono::DateTime::parse_from_rfc3339(&created_at)
                            .map_or(0.0, |dt| {
                                (now - dt.with_timezone(&Utc)).num_seconds() as f64 / 86400.0
                            });
                        let decayed = normalized * recency_decay(age_days, recency_half_life_days);
                        fts_scores.insert(id, (decayed, key, content));
                    }
                }
            }
        }

        // 2. Get vector similarity scores (from in-memory cache)
        let mut vec_scores: std::collections::HashMap<i64, (f32, String, String)> =
            std::collections::HashMap::new();

        if keyword_weight < 1.0 {
            let cached = self.get_cached_embeddings(exclude_sources)?;
            for entry in &cached {
                let sim = cosine_similarity(query_embedding, &entry.embedding);
                // Cosine similarity is already in [-1, 1]; clamp to [0, 1]
                let score = sim.max(0.0);
                vec_scores.insert(
                    entry.entry_id,
                    (score, entry.source_key.clone(), entry.content.clone()),
                );
            }
        }

        // 3. Merge scores using the configured fusion strategy
        let mut all_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
        all_ids.extend(fts_scores.keys());
        all_ids.extend(vec_scores.keys());

        let mut scored: Vec<(f32, String, String)> = match fusion_strategy {
            FusionStrategy::WeightedScore => all_ids
                .into_iter()
                .map(|id| {
                    let (fts_score, fts_key, fts_content) = fts_scores
                        .get(&id)
                        .cloned()
                        .unwrap_or((0.0, String::new(), String::new()));
                    let (vec_score, vec_key, vec_content) = vec_scores
                        .get(&id)
                        .cloned()
                        .unwrap_or((0.0, String::new(), String::new()));

                    let combined = keyword_weight * fts_score + (1.0 - keyword_weight) * vec_score;
                    let key = if !fts_key.is_empty() {
                        fts_key
                    } else if !vec_key.is_empty() {
                        vec_key
                    } else {
                        "<unknown>".to_string()
                    };
                    let content = if fts_content.is_empty() {
                        vec_content
                    } else {
                        fts_content
                    };
                    (combined, key, content)
                })
                .collect(),

            FusionStrategy::Rrf => {
                // Reciprocal Rank Fusion: score = 1/(k+rank_fts) + 1/(k+rank_vec)
                // Rank by descending score; items absent from a list get rank = list_size + 1
                let k = rrf_k.max(1) as f32;

                // Build FTS rank map (1-indexed, sorted by score descending)
                let mut fts_ranked: Vec<(i64, f32)> =
                    fts_scores.iter().map(|(id, (s, _, _))| (*id, *s)).collect();
                fts_ranked
                    .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                let fts_rank_map: std::collections::HashMap<i64, usize> = fts_ranked
                    .iter()
                    .enumerate()
                    .map(|(rank, (id, _))| (*id, rank + 1))
                    .collect();
                let fts_absent_rank = fts_ranked.len() + 1;

                // Build vector rank map
                let mut vec_ranked: Vec<(i64, f32)> =
                    vec_scores.iter().map(|(id, (s, _, _))| (*id, *s)).collect();
                vec_ranked
                    .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                let vec_rank_map: std::collections::HashMap<i64, usize> = vec_ranked
                    .iter()
                    .enumerate()
                    .map(|(rank, (id, _))| (*id, rank + 1))
                    .collect();
                let vec_absent_rank = vec_ranked.len() + 1;

                all_ids
                    .into_iter()
                    .map(|id| {
                        let fts_rank = fts_rank_map.get(&id).copied().unwrap_or(fts_absent_rank);
                        let vec_rank = vec_rank_map.get(&id).copied().unwrap_or(vec_absent_rank);
                        let rrf_score = 1.0 / (k + fts_rank as f32) + 1.0 / (k + vec_rank as f32);

                        let (_, fts_key, fts_content) = fts_scores.get(&id).cloned().unwrap_or((
                            0.0,
                            String::new(),
                            String::new(),
                        ));
                        let (_, vec_key, vec_content) = vec_scores.get(&id).cloned().unwrap_or((
                            0.0,
                            String::new(),
                            String::new(),
                        ));

                        let key = if !fts_key.is_empty() {
                            fts_key
                        } else if !vec_key.is_empty() {
                            vec_key
                        } else {
                            "<unknown>".to_string()
                        };
                        let content = if fts_content.is_empty() {
                            vec_content
                        } else {
                            fts_content
                        };
                        (rrf_score, key, content)
                    })
                    .collect()
            }
        };

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let top_score = scored.first().map(|(s, _, _)| f64::from(*s));
        let hits: Vec<MemoryHit> = scored
            .into_iter()
            .take(limit)
            .map(|(_, source_key, content)| MemoryHit {
                source_key,
                content,
            })
            .collect();

        if let Err(e) = self.log_search(query_text, "hybrid", &hits, top_score) {
            debug!("failed to log hybrid search: {}", e);
        }

        Ok(hits)
    }

    /// Return entry IDs and content for a given source key (for embedding generation).
    pub fn get_entries_for_source(&self, source_key: &str) -> Result<Vec<(i64, String)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let mut stmt =
            conn.prepare("SELECT id, content FROM memory_entries WHERE source_key = ?")?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map([source_key], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect();
        rows.map_err(|e| anyhow::anyhow!("Failed to get entries: {}", e))
    }

    /// List all source keys in the database.
    pub fn list_source_keys(&self) -> Result<Vec<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let mut stmt = conn.prepare("SELECT source_key FROM memory_sources")?;
        let keys: Result<Vec<_>, _> = stmt.query_map([], |row| row.get(0))?.collect();
        keys.map_err(|e| anyhow::anyhow!("Failed to list source keys: {}", e))
    }

    /// Remove a source and all its entries from the database.
    pub fn remove_source(&self, source_key: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        // Explicitly delete embeddings (CASCADE requires PRAGMA foreign_keys=ON
        // which may not have been set on older databases)
        conn.execute(
            "DELETE FROM memory_embeddings WHERE entry_id IN \
             (SELECT id FROM memory_entries WHERE source_key = ?)",
            [source_key],
        )?;
        conn.execute(
            "DELETE FROM memory_entries WHERE source_key = ?",
            [source_key],
        )?;
        conn.execute(
            "DELETE FROM memory_sources WHERE source_key = ?",
            [source_key],
        )?;
        Ok(())
    }

    /// Log a search query and the source keys it returned.
    pub fn log_search(
        &self,
        query: &str,
        search_type: &str,
        results: &[MemoryHit],
        top_score: Option<f64>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        conn.execute(
            "INSERT INTO memory_access_log (query, search_type, result_count, top_score)
             VALUES (?, ?, ?, ?)",
            params![query, search_type, results.len() as i64, top_score],
        )?;
        let log_id = conn.last_insert_rowid();
        for hit in results {
            conn.execute(
                "INSERT INTO memory_search_hits (access_log_id, source_key) VALUES (?, ?)",
                params![log_id, hit.source_key],
            )?;
        }
        Ok(())
    }

    /// Count how many times a source key appeared in search results.
    pub fn get_source_hit_count(&self, source_key: &str) -> Result<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_search_hits WHERE source_key = ?",
                [source_key],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(count as u64)
    }

    /// Get entries that have no embeddings (for back-fill).
    pub fn get_entries_missing_embeddings(&self) -> Result<Vec<(i64, String, String)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT e.id, e.source_key, e.content FROM memory_entries e
             LEFT JOIN memory_embeddings em ON e.id = em.entry_id
             WHERE em.entry_id IS NULL",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect();
        rows.map_err(|e| anyhow::anyhow!("failed to get entries missing embeddings: {}", e))
    }

    /// Record an LLM cost entry.
    #[allow(clippy::too_many_arguments)]
    pub fn record_cost(
        &self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_tokens: u64,
        cache_read_tokens: u64,
        cost_cents: f64,
        caller: &str,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        conn.execute(
            "INSERT INTO llm_cost_log
             (model, input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens, cost_cents, caller)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                model,
                input_tokens as i64,
                output_tokens as i64,
                cache_creation_tokens as i64,
                cache_read_tokens as i64,
                cost_cents,
                caller,
            ],
        )?;
        Ok(())
    }

    /// Get total cost in cents for a given date (YYYY-MM-DD).
    pub fn get_daily_cost(&self, date_str: &str) -> Result<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let pattern = format!("{}%", date_str);
        let total: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_cents), 0.0) FROM llm_cost_log WHERE timestamp LIKE ?",
                [&pattern],
                |row| row.get(0),
            )
            .unwrap_or(0.0);
        Ok(total)
    }

    /// Get cost summary grouped by date and model since a given date (YYYY-MM-DD).
    pub fn get_cost_summary(&self, since_date: &str) -> Result<Vec<CostSummaryRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT DATE(timestamp) as day, model,
                    SUM(cost_cents) as total_cents,
                    SUM(input_tokens) as total_input,
                    SUM(output_tokens) as total_output,
                    COUNT(*) as call_count
             FROM llm_cost_log
             WHERE DATE(timestamp) >= ?
             GROUP BY day, model
             ORDER BY day DESC, total_cents DESC",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map([since_date], |row| {
                Ok(CostSummaryRow {
                    date: row.get(0)?,
                    model: row.get(1)?,
                    total_cents: row.get(2)?,
                    total_input_tokens: row.get(3)?,
                    total_output_tokens: row.get(4)?,
                    call_count: row.get(5)?,
                })
            })?
            .collect();
        rows.map_err(|e| anyhow::anyhow!("failed to get cost summary: {}", e))
    }

    /// Get search log stats: total searches, total hits, unique queries.
    pub fn get_search_stats(&self) -> Result<SearchStats> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let total_searches: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_access_log", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        let total_hits: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_search_hits", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        let avg_results: f64 = conn
            .query_row(
                "SELECT COALESCE(AVG(result_count), 0.0) FROM memory_access_log",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0.0);
        Ok(SearchStats {
            total_searches: total_searches as u64,
            total_hits: total_hits as u64,
            avg_results_per_search: avg_results,
        })
    }

    /// Get top source keys by search hit count.
    pub fn get_top_sources(&self, limit: usize) -> Result<Vec<(String, u64)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT source_key, COUNT(*) as hits FROM memory_search_hits
             GROUP BY source_key ORDER BY hits DESC LIMIT ?",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map([limit as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })?
            .collect();
        rows.map_err(|e| anyhow::anyhow!("failed to get top sources: {}", e))
    }

    /// Record an intent classification or hallucination detection event.
    ///
    /// - `event_type`: `"classification"` or `"hallucination"`
    /// - `intent_method`: `"regex"`, `"semantic"`, or `"none"` (for classification)
    /// - `semantic_score`: cosine similarity score (if semantic classifier ran)
    /// - `detection_layer`: `"layer0_false_no_tools"`, `"layer1_regex"`, `"layer2_intent"`
    /// - `message_preview`: first 100 chars of user message or LLM response
    pub fn record_intent_event(
        &self,
        event_type: &str,
        intent_method: Option<&str>,
        semantic_score: Option<f32>,
        detection_layer: Option<&str>,
        message_preview: &str,
    ) -> Result<()> {
        let preview = &message_preview[..message_preview
            .char_indices()
            .nth(100)
            .map_or(message_preview.len(), |(i, _)| i)];
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        conn.execute(
            "INSERT INTO intent_metrics
             (event_type, intent_method, semantic_score, detection_layer, message_preview)
             VALUES (?, ?, ?, ?, ?)",
            params![
                event_type,
                intent_method,
                semantic_score,
                detection_layer,
                preview,
            ],
        )?;
        Ok(())
    }

    /// Get intent metrics summary for the given period.
    pub fn get_intent_stats(&self, since_date: &str) -> Result<IntentStats> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;

        let total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM intent_metrics WHERE event_type = 'classification' AND DATE(timestamp) >= ?",
                [since_date],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let regex_action: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM intent_metrics WHERE event_type = 'classification' AND intent_method = 'regex' AND DATE(timestamp) >= ?",
                [since_date],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let semantic_action: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM intent_metrics WHERE event_type = 'classification' AND intent_method = 'semantic' AND DATE(timestamp) >= ?",
                [since_date],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let not_action: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM intent_metrics WHERE event_type = 'classification' AND intent_method = 'none' AND DATE(timestamp) >= ?",
                [since_date],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let hallucinations: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM intent_metrics WHERE event_type = 'hallucination' AND DATE(timestamp) >= ?",
                [since_date],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let layer1_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM intent_metrics WHERE event_type = 'hallucination' AND detection_layer = 'layer1_regex' AND DATE(timestamp) >= ?",
                [since_date],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let layer2_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM intent_metrics WHERE event_type = 'hallucination' AND detection_layer = 'layer2_intent' AND DATE(timestamp) >= ?",
                [since_date],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let avg_semantic_action: f64 = conn
            .query_row(
                "SELECT COALESCE(AVG(semantic_score), 0.0) FROM intent_metrics
                 WHERE event_type = 'classification' AND intent_method = 'semantic' AND DATE(timestamp) >= ?",
                [since_date],
                |row| row.get(0),
            )
            .unwrap_or(0.0);

        let avg_semantic_non_action: f64 = conn
            .query_row(
                "SELECT COALESCE(AVG(semantic_score), 0.0) FROM intent_metrics
                 WHERE event_type = 'classification' AND intent_method = 'none' AND semantic_score IS NOT NULL AND DATE(timestamp) >= ?",
                [since_date],
                |row| row.get(0),
            )
            .unwrap_or(0.0);

        Ok(IntentStats {
            total_classified: total as u64,
            regex_action: regex_action as u64,
            semantic_action: semantic_action as u64,
            not_action: not_action as u64,
            hallucinations_caught: hallucinations as u64,
            layer1_regex: layer1_count as u64,
            layer2_intent: layer2_count as u64,
            avg_semantic_score_action: avg_semantic_action,
            avg_semantic_score_non_action: avg_semantic_non_action,
        })
    }

    /// Get recent hallucination events for inspection.
    pub fn get_recent_hallucinations(&self, limit: usize) -> Result<Vec<IntentEvent>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT timestamp, detection_layer, message_preview
             FROM intent_metrics WHERE event_type = 'hallucination'
             ORDER BY timestamp DESC LIMIT ?",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map([limit as i64], |row| {
                Ok(IntentEvent {
                    timestamp: row.get(0)?,
                    detection_layer: row.get(1)?,
                    message_preview: row.get(2)?,
                })
            })?
            .collect();
        rows.map_err(|e| anyhow::anyhow!("failed to get recent hallucinations: {}", e))
    }

    /// Purge search access logs older than `days`. Returns number of rows deleted.
    /// Also cleans up orphaned `memory_search_hits` referencing deleted logs.
    pub fn purge_old_search_logs(&self, days: u32) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(days));
        let cutoff_str = cutoff.format("%Y-%m-%d %H:%M:%S").to_string();
        // Delete orphaned hits first (FK has no CASCADE)
        conn.execute(
            "DELETE FROM memory_search_hits WHERE access_log_id IN (
                 SELECT id FROM memory_access_log WHERE created_at < ?
             )",
            [&cutoff_str],
        )?;
        let deleted = conn.execute(
            "DELETE FROM memory_access_log WHERE created_at < ?",
            [&cutoff_str],
        )?;
        Ok(deleted)
    }

    pub fn search(
        &self,
        query_text: &str,
        limit: usize,
        exclude_sources: Option<&std::collections::HashSet<String>>,
    ) -> Result<Vec<MemoryHit>> {
        let hits = self.search_inner(query_text, limit, exclude_sources)?;
        // Log search asynchronously (best-effort, don't fail the search)
        if let Err(e) = self.log_search(query_text, "keyword", &hits, None) {
            debug!("failed to log search: {}", e);
        }
        Ok(hits)
    }

    fn search_inner(
        &self,
        query_text: &str,
        limit: usize,
        exclude_sources: Option<&std::collections::HashSet<String>>,
    ) -> Result<Vec<MemoryHit>> {
        let query = fts_query(query_text);
        if query.is_empty() {
            return Ok(vec![]);
        }

        let default_set = std::collections::HashSet::new();
        let exclude = exclude_sources.unwrap_or(&default_set);
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;

        if self.has_fts {
            let mut stmt = conn.prepare(
                "SELECT me.source_key, me.content
                FROM memory_fts
                JOIN memory_entries me ON memory_fts.rowid = me.id
                WHERE memory_fts MATCH ?
                ORDER BY bm25(memory_fts)
                LIMIT ?",
            )?;

            let rows: Result<Vec<_>, _> = stmt
                .query_map([&query, &(limit + exclude.len()).to_string()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect();

            match rows {
                Ok(rows) => {
                    let hits: Vec<MemoryHit> = rows
                        .into_iter()
                        .filter(|(key, _)| !exclude.contains(key))
                        .take(limit)
                        .map(|(source_key, content)| MemoryHit {
                            source_key,
                            content,
                        })
                        .collect();
                    return Ok(hits);
                }
                Err(e) => {
                    warn!("FTS5 query failed, falling back to LIKE: {}", e);
                }
            }
        }

        // Fallback: LIKE search
        let like = format!(
            "%{}%",
            query_text.trim().chars().take(200).collect::<String>()
        );
        let mut stmt = conn.prepare(
            "SELECT source_key, content
            FROM memory_entries
            WHERE content LIKE ?
            LIMIT ?",
        )?;

        let rows: Result<Vec<_>, _> = stmt
            .query_map([&like, &(limit + exclude.len()).to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect();

        if let Ok(rows) = rows {
            let hits: Vec<MemoryHit> = rows
                .into_iter()
                .filter(|(key, _)| !exclude.contains(key))
                .take(limit)
                .map(|(source_key, content)| MemoryHit {
                    source_key,
                    content,
                })
                .collect();
            return Ok(hits);
        }

        Ok(vec![])
    }

    // ── DLQ (Dead Letter Queue) methods ─────────────────────────

    pub fn insert_dlq_entry(
        &self,
        job_id: &str,
        job_name: &str,
        payload: &str,
        error_message: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT INTO scheduled_task_dlq (job_id, job_name, payload, error_message)
             VALUES (?1, ?2, ?3, ?4)",
            params![job_id, job_name, payload, error_message],
        )?;
        let id = conn.last_insert_rowid();

        // Auto-purge: keep only 100 most recent entries
        conn.execute(
            "DELETE FROM scheduled_task_dlq WHERE id NOT IN (
                SELECT id FROM scheduled_task_dlq ORDER BY id DESC LIMIT 100
            )",
            [],
        )?;

        Ok(id)
    }

    pub fn list_dlq_entries(&self, status_filter: Option<&str>) -> Result<Vec<DlqEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let (sql, filter_params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(
            status,
        ) =
            status_filter
        {
            (
                    "SELECT id, job_id, job_name, payload, error_message, failed_at, retry_count, status
                     FROM scheduled_task_dlq WHERE status = ?1 ORDER BY id DESC",
                    vec![Box::new(status.to_string())],
                )
        } else {
            (
                    "SELECT id, job_id, job_name, payload, error_message, failed_at, retry_count, status
                     FROM scheduled_task_dlq ORDER BY id DESC",
                    vec![],
                )
        };

        let mut stmt = conn.prepare(sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            filter_params.iter().map(AsRef::as_ref).collect();
        let rows = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok(DlqEntry {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    job_name: row.get(2)?,
                    payload: row.get(3)?,
                    error_message: row.get(4)?,
                    failed_at: row.get(5)?,
                    retry_count: row.get(6)?,
                    status: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    pub fn update_dlq_status(&self, id: i64, new_status: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let updated = conn.execute(
            "UPDATE scheduled_task_dlq SET status = ?1 WHERE id = ?2",
            params![new_status, id],
        )?;
        Ok(updated > 0)
    }

    pub fn increment_dlq_retry(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let updated = conn.execute(
            "UPDATE scheduled_task_dlq SET retry_count = retry_count + 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(updated > 0)
    }

    pub fn clear_dlq(&self, status_filter: Option<&str>) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let deleted = if let Some(status) = status_filter {
            conn.execute(
                "DELETE FROM scheduled_task_dlq WHERE status = ?1",
                params![status],
            )?
        } else {
            conn.execute("DELETE FROM scheduled_task_dlq", [])?
        };
        Ok(deleted)
    }

    // ── Workspace file manifest methods ──────────────────────────

    /// Register a workspace file in the manifest (upsert).
    #[allow(clippy::too_many_arguments)]
    pub fn register_workspace_file(
        &self,
        path: &str,
        category: &str,
        original_name: Option<&str>,
        size_bytes: i64,
        source_tool: Option<&str>,
        session_key: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT INTO workspace_files
               (path, category, original_name, size_bytes, source_tool, session_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
             ON CONFLICT(path) DO UPDATE SET
               category = excluded.category,
               original_name = excluded.original_name,
               size_bytes = excluded.size_bytes,
               source_tool = excluded.source_tool,
               session_key = excluded.session_key",
            params![
                path,
                category,
                original_name,
                size_bytes,
                source_tool,
                session_key
            ],
        )?;
        Ok(())
    }

    /// Register a workspace file with an explicit `created_at` timestamp (for testing).
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn register_workspace_file_with_date(
        &self,
        path: &str,
        category: &str,
        original_name: Option<&str>,
        size_bytes: i64,
        source_tool: Option<&str>,
        session_key: Option<&str>,
        created_at: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT INTO workspace_files
               (path, category, original_name, size_bytes, source_tool, session_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(path) DO UPDATE SET
               category = excluded.category,
               original_name = excluded.original_name,
               size_bytes = excluded.size_bytes,
               source_tool = excluded.source_tool,
               session_key = excluded.session_key,
               created_at = excluded.created_at",
            params![
                path,
                category,
                original_name,
                size_bytes,
                source_tool,
                session_key,
                created_at
            ],
        )?;
        Ok(())
    }

    /// Remove a workspace file from the manifest.
    pub fn unregister_workspace_file(&self, path: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute("DELETE FROM workspace_files WHERE path = ?1", params![path])?;
        Ok(())
    }

    /// List workspace files with optional filters.
    pub fn list_workspace_files(
        &self,
        category: Option<&str>,
        date: Option<&str>,
        tag: Option<&str>,
    ) -> Result<Vec<WorkspaceFileEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;

        let mut sql = String::from(
            "SELECT id, path, category, original_name, size_bytes, source_tool, tags, created_at, accessed_at, session_key
             FROM workspace_files WHERE 1=1",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(cat) = category {
            let _ = write!(sql, " AND category = ?{}", param_values.len() + 1);
            param_values.push(Box::new(cat.to_string()));
        }
        if let Some(d) = date {
            let _ = write!(sql, " AND created_at LIKE ?{}", param_values.len() + 1);
            param_values.push(Box::new(format!("{d}%")));
        }
        if let Some(t) = tag {
            let _ = write!(
                sql,
                " AND (',' || tags || ',' LIKE '%,' || ?{} || ',%')",
                param_values.len() + 1
            );
            param_values.push(Box::new(t.to_string()));
        }
        sql.push_str(" ORDER BY created_at DESC");

        let mut stmt = conn.prepare(&sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(AsRef::as_ref).collect();
        let rows = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok(WorkspaceFileEntry {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    category: row.get(2)?,
                    original_name: row.get(3)?,
                    size_bytes: row.get(4)?,
                    source_tool: row.get(5)?,
                    tags: row.get(6)?,
                    created_at: row.get(7)?,
                    accessed_at: row.get(8)?,
                    session_key: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Search workspace files by path or original name.
    pub fn search_workspace_files(&self, query: &str) -> Result<Vec<WorkspaceFileEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let pattern = format!("%{query}%");
        let mut stmt = conn.prepare(
            "SELECT id, path, category, original_name, size_bytes, source_tool, tags, created_at, accessed_at, session_key
             FROM workspace_files
             WHERE path LIKE ?1 OR original_name LIKE ?1
             ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![pattern], |row| {
                Ok(WorkspaceFileEntry {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    category: row.get(2)?,
                    original_name: row.get(3)?,
                    size_bytes: row.get(4)?,
                    source_tool: row.get(5)?,
                    tags: row.get(6)?,
                    created_at: row.get(7)?,
                    accessed_at: row.get(8)?,
                    session_key: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Update `accessed_at` timestamp for a workspace file.
    pub fn touch_workspace_file(&self, path: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "UPDATE workspace_files SET accessed_at = datetime('now') WHERE path = ?1",
            params![path],
        )?;
        Ok(())
    }

    /// Set tags on a workspace file.
    pub fn set_workspace_file_tags(&self, path: &str, tags: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "UPDATE workspace_files SET tags = ?1 WHERE path = ?2",
            params![tags, path],
        )?;
        Ok(())
    }

    /// List workspace files that have exceeded their TTL.
    pub fn list_expired_workspace_files(
        &self,
        category: &str,
        ttl_days: u32,
    ) -> Result<Vec<WorkspaceFileEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let modifier = format!("-{ttl_days} days");
        let mut stmt = conn.prepare(
            "SELECT id, path, category, original_name, size_bytes, source_tool, tags, created_at, accessed_at, session_key
             FROM workspace_files
             WHERE category = ?1 AND created_at < datetime('now', ?2)
             ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![category, modifier], |row| {
                Ok(WorkspaceFileEntry {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    category: row.get(2)?,
                    original_name: row.get(3)?,
                    size_bytes: row.get(4)?,
                    source_tool: row.get(5)?,
                    tags: row.get(6)?,
                    created_at: row.get(7)?,
                    accessed_at: row.get(8)?,
                    session_key: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Move a workspace file to a new path and category.
    pub fn move_workspace_file(
        &self,
        old_path: &str,
        new_path: &str,
        new_category: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "UPDATE workspace_files SET path = ?1, category = ?2 WHERE path = ?3",
            params![new_path, new_category, old_path],
        )?;
        Ok(())
    }
}

fn hash_text(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hex::encode(hasher.finalize())
}

fn split_into_chunks(text: &str) -> Vec<String> {
    let re = crate::utils::regex::RegexPatterns::double_newlines();
    let raw: Vec<&str> = re.split(text.trim()).collect();
    let mut chunks = Vec::new();

    for part in raw {
        let p = part.trim();
        if p.is_empty() || p.len() < MIN_CHUNK_SIZE {
            continue;
        }
        let chunk = if p.len() > MAX_CHUNK_SIZE {
            let mut end = MAX_CHUNK_SIZE;
            while end > 0 && !p.is_char_boundary(end) {
                end -= 1;
            }
            p[..end].to_string()
        } else {
            p.to_string()
        };
        chunks.push(chunk);
    }

    chunks
}

/// Strip HTML tags and return plain text content.
fn strip_html_tags(html: &str) -> String {
    let document = scraper::Html::parse_document(html);
    let mut text = String::with_capacity(html.len() / 2);
    for node in document.tree.values() {
        if let scraper::node::Node::Text(t) = node {
            let s = t.text.trim();
            if !s.is_empty() {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(s);
            }
        }
    }
    text
}

fn fts_query(text: &str) -> String {
    let re = crate::utils::regex::RegexPatterns::words();
    let terms: Vec<&str> = re.find_iter(text).map(|m| m.as_str()).collect();

    if terms.is_empty() {
        return String::new();
    }

    let mut seen = std::collections::HashSet::new();
    let mut unique = Vec::new();

    for term in terms {
        let low = term.to_lowercase();
        if !seen.contains(&low) {
            seen.insert(low.clone());
            unique.push(low);
        }
        if unique.len() >= MAX_FTS_TERMS {
            break;
        }
    }

    // Double-quote each term to prevent FTS5 operator injection
    // (e.g. user searching for "NOT important" won't trigger NOT operator)
    unique
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

#[cfg(test)]
mod tests;
