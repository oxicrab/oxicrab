use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::path::Path;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct MemoryHit {
    pub source_key: String,
    pub content: String,
}

/// Minimum size for a memory chunk (paragraphs shorter than this are skipped)
const MIN_CHUNK_SIZE: usize = 12;
/// Maximum size for a memory chunk (longer paragraphs are truncated)
const MAX_CHUNK_SIZE: usize = 1200;
/// Maximum number of unique terms used in FTS queries
const MAX_FTS_TERMS: usize = 16;

pub struct MemoryDB {
    conn: std::sync::Mutex<Connection>,
    db_path: String,
    has_fts: bool,
}

impl Clone for MemoryDB {
    fn clone(&self) -> Self {
        // Re-open a connection for clones (rare, needed for spawn_blocking patterns).
        let new_conn = Connection::open(&self.db_path)
            .expect("Failed to re-open DB for clone; path was valid at construction time");
        new_conn
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA busy_timeout=3000;",
            )
            .expect("Failed to set PRAGMAs on cloned connection");
        Self {
            conn: std::sync::Mutex::new(new_conn),
            db_path: self.db_path.clone(),
            has_fts: self.has_fts,
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
             PRAGMA busy_timeout=3000;",
        )?;

        let mut db = Self {
            conn: std::sync::Mutex::new(conn),
            db_path: db_path.to_string_lossy().to_string(),
            has_fts: false,
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
            debug!("FTS5 not available; falling back to LIKE");
        }

        Ok(())
    }

    fn get_mtime_ns(path: &Path) -> i64 {
        path.metadata()
            .and_then(|m| {
                m.modified().map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_nanos() as i64)
                })
            })
            .unwrap_or(0)
    }

    pub fn index_file(&self, source_key: &str, path: &Path) -> Result<()> {
        let mtime_ns = Self::get_mtime_ns(path);
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

        if let Some(existing_mtime) = existing {
            if existing_mtime == mtime_ns {
                return Ok(()); // unchanged
            }
        }

        // Wipe old entries
        conn.execute(
            "DELETE FROM memory_entries WHERE source_key = ?",
            [source_key],
        )?;

        // Read and index file
        let text = if path.exists() && path.is_file() {
            std::fs::read_to_string(path).unwrap_or_default()
        } else {
            String::new()
        };

        for chunk in split_into_chunks(&text) {
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
            params![source_key, mtime_ns, now],
        )?;

        debug!("Indexed memory file {}", source_key);
        Ok(())
    }

    pub fn index_directory(&self, memory_dir: &Path) -> Result<()> {
        if !memory_dir.is_dir() {
            return Ok(());
        }

        for entry in std::fs::read_dir(memory_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension() == Some(std::ffi::OsStr::new("md")) {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    self.index_file(name, &path)?;
                }
            }
        }

        Ok(())
    }

    /// Store an embedding for a memory entry.
    pub fn store_embedding(&self, entry_id: i64, embedding: &[u8]) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        conn.execute(
            "INSERT OR REPLACE INTO memory_embeddings (entry_id, embedding) VALUES (?, ?)",
            params![entry_id, embedding],
        )?;
        Ok(())
    }

    /// Get all embeddings, optionally excluding certain source keys.
    /// Returns (`entry_id`, content, `embedding_blob`).
    pub fn get_all_embeddings(
        &self,
        exclude_sources: Option<&std::collections::HashSet<String>>,
    ) -> Result<Vec<(i64, String, Vec<u8>)>> {
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
            .map(|(id, content, _, emb)| (id, content, emb))
            .collect())
    }

    /// Hybrid search combining FTS5 BM25 and vector cosine similarity.
    /// `keyword_weight` controls blending: 1.0 = keyword only, 0.0 = vector only.
    pub fn hybrid_search(
        &self,
        query_text: &str,
        query_embedding: &[f32],
        limit: usize,
        exclude_sources: Option<&std::collections::HashSet<String>>,
        keyword_weight: f32,
    ) -> Result<Vec<MemoryHit>> {
        use crate::agent::memory::embeddings::{cosine_similarity, deserialize_embedding};

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
                    "SELECT me.id, me.source_key, me.content, bm25(memory_fts) as score
                     FROM memory_fts
                     JOIN memory_entries me ON memory_fts.rowid = me.id
                     WHERE memory_fts MATCH ?
                     ORDER BY bm25(memory_fts)
                     LIMIT 100",
                )?;

                let rows: Vec<_> = stmt
                    .query_map([&query], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, f64>(3)?,
                        ))
                    })?
                    .filter_map(std::result::Result::ok)
                    .filter(|(_, key, _, _)| !exclude.contains(key))
                    .collect();

                // BM25 scores are negative (more negative = better match).
                // Normalize to 0..1 range.
                if !rows.is_empty() {
                    let min_score = rows
                        .iter()
                        .map(|(_, _, _, s)| *s)
                        .fold(f64::INFINITY, f64::min);
                    let max_score = rows
                        .iter()
                        .map(|(_, _, _, s)| *s)
                        .fold(f64::NEG_INFINITY, f64::max);
                    let range = max_score - min_score;

                    for (id, key, content, score) in rows {
                        let normalized = if range.abs() < 1e-10 {
                            1.0
                        } else {
                            // Invert: most negative (best) -> 1.0, least negative (worst) -> 0.0
                            ((max_score - score) / range) as f32
                        };
                        fts_scores.insert(id, (normalized, key, content));
                    }
                }
            }
        }

        // 2. Get vector similarity scores
        let mut vec_scores: std::collections::HashMap<i64, (f32, String, String)> =
            std::collections::HashMap::new();

        if keyword_weight < 1.0 {
            let all_embeddings = self.get_all_embeddings(exclude_sources)?;
            for (entry_id, content, emb_bytes) in &all_embeddings {
                let emb = deserialize_embedding(emb_bytes);
                let sim = cosine_similarity(query_embedding, &emb);
                // Cosine similarity is already in [-1, 1]; clamp to [0, 1]
                let score = sim.max(0.0);
                // We need source_key for display — extract from FTS scores or DB
                // For simplicity, use content as lookup key
                vec_scores.insert(*entry_id, (score, String::new(), content.clone()));
            }

            // Fill in source keys from DB if needed
            if !vec_scores.is_empty() {
                let conn = self
                    .conn
                    .lock()
                    .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
                let entry_ids: Vec<i64> = vec_scores.keys().copied().collect();
                for entry_id in entry_ids {
                    if let Ok(key) = conn.query_row(
                        "SELECT source_key FROM memory_entries WHERE id = ?",
                        [entry_id],
                        |row| row.get::<_, String>(0),
                    ) {
                        if let Some(entry) = vec_scores.get_mut(&entry_id) {
                            entry.1 = key;
                        }
                    }
                }
            }
        }

        // 3. Merge scores
        let mut all_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
        all_ids.extend(fts_scores.keys());
        all_ids.extend(vec_scores.keys());

        let mut scored: Vec<(f32, String, String)> =
            all_ids
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
                    let key = if fts_key.is_empty() { vec_key } else { fts_key };
                    let content = if fts_content.is_empty() {
                        vec_content
                    } else {
                        fts_content
                    };
                    (combined, key, content)
                })
                .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        Ok(scored
            .into_iter()
            .take(limit)
            .map(|(_, source_key, content)| MemoryHit {
                source_key,
                content,
            })
            .collect())
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

    pub fn search(
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

    unique.join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_text_deterministic() {
        let h1 = hash_text("hello world");
        let h2 = hash_text("hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_text_different_inputs() {
        let h1 = hash_text("hello");
        let h2 = hash_text("world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_split_into_chunks_skips_short() {
        let text = "short\n\nalso short\n\nthis is long enough to be a chunk";
        let chunks = split_into_chunks(text);
        // "short" and "also short" are < MIN_CHUNK_SIZE (12), should be skipped
        assert!(!chunks.iter().any(|c| c == "short"));
        assert!(!chunks.iter().any(|c| c == "also short"));
    }

    #[test]
    fn test_split_into_chunks_truncates_long() {
        let long_paragraph = "a".repeat(2000);
        let chunks = split_into_chunks(&long_paragraph);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].len() <= MAX_CHUNK_SIZE);
    }

    #[test]
    fn test_split_into_chunks_utf8_safe_truncation() {
        // Create a string of multi-byte chars longer than MAX_CHUNK_SIZE
        let text = "\u{1F600}".repeat(500); // 500 * 4 = 2000 bytes
        let chunks = split_into_chunks(&text);
        // Each chunk should be valid UTF-8
        for chunk in &chunks {
            for c in chunk.chars() {
                assert_eq!(c, '\u{1F600}');
            }
        }
    }

    #[test]
    fn test_split_into_chunks_normal_paragraphs() {
        let text =
            "This is paragraph one with enough text.\n\nThis is paragraph two with enough text.";
        let chunks = split_into_chunks(text);
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn test_split_into_chunks_empty_input() {
        let chunks = split_into_chunks("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_fts_query_simple() {
        let q = fts_query("hello world");
        assert!(q.contains("hello"));
        assert!(q.contains("world"));
        assert!(q.contains(" OR "));
    }

    #[test]
    fn test_fts_query_empty() {
        assert_eq!(fts_query(""), "");
    }

    #[test]
    fn test_fts_query_deduplicates() {
        let q = fts_query("hello hello hello");
        // Should only have "hello" once
        assert_eq!(q.matches("hello").count(), 1);
    }

    #[test]
    fn test_fts_query_case_insensitive() {
        let q = fts_query("Hello HELLO hello");
        assert_eq!(q.matches("hello").count(), 1);
    }

    #[test]
    fn test_fts_query_max_terms() {
        let terms: Vec<String> = (0..30).map(|i| format!("word{}", i)).collect();
        let q = fts_query(&terms.join(" "));
        let count = q.split(" OR ").count();
        assert!(count <= MAX_FTS_TERMS);
    }

    #[test]
    fn test_fts_query_symbols_stripped() {
        let q = fts_query("!!! ??? ...");
        assert_eq!(q, "");
    }

    #[test]
    fn test_memory_db_new_creates_schema() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let db = MemoryDB::new(&db_path).unwrap();
        // Should be able to search without error
        let results = db.search("anything", 10, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_memory_db_index_and_search() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_memory.db");

        // Write a test file
        let test_file = dir.path().join("notes.md");
        std::fs::write(
            &test_file,
            "This is a test document about Rust programming.\n\n\
             Another paragraph about async runtime and tokio.",
        )
        .unwrap();

        let db = MemoryDB::new(&db_path).unwrap();
        db.index_file("notes.md", &test_file).unwrap();

        let results = db.search("Rust programming", 10, None).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("Rust"));
    }

    #[test]
    fn test_memory_db_search_no_results() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_memory.db");

        let test_file = dir.path().join("notes.md");
        std::fs::write(
            &test_file,
            "This document is about cooking recipes.\n\nAnother paragraph about food.",
        )
        .unwrap();

        let db = MemoryDB::new(&db_path).unwrap();
        db.index_file("notes.md", &test_file).unwrap();

        let results = db.search("quantum physics", 10, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_memory_db_exclude_sources() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_memory.db");

        let file1 = dir.path().join("notes1.md");
        std::fs::write(&file1, "This is about Rust programming language.").unwrap();
        let file2 = dir.path().join("notes2.md");
        std::fs::write(&file2, "This is also about Rust async patterns.").unwrap();

        let db = MemoryDB::new(&db_path).unwrap();
        db.index_file("notes1.md", &file1).unwrap();
        db.index_file("notes2.md", &file2).unwrap();

        let mut exclude = std::collections::HashSet::new();
        exclude.insert("notes1.md".to_string());

        let results = db.search("Rust", 10, Some(&exclude)).unwrap();
        // notes1.md should be excluded
        for hit in &results {
            assert_ne!(hit.source_key, "notes1.md");
        }
    }

    #[test]
    fn test_memory_db_reindex_unchanged_file_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_memory.db");

        let test_file = dir.path().join("notes.md");
        std::fs::write(
            &test_file,
            "Content about database indexing and memory systems.",
        )
        .unwrap();

        let db = MemoryDB::new(&db_path).unwrap();
        db.index_file("notes.md", &test_file).unwrap();
        // Indexing the same unchanged file again should be a no-op
        db.index_file("notes.md", &test_file).unwrap();

        let results = db.search("database", 10, None).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_memory_db_index_directory() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let memory_dir = dir.path().join("memory");
        std::fs::create_dir(&memory_dir).unwrap();

        std::fs::write(
            memory_dir.join("file1.md"),
            "This file is about artificial intelligence and machine learning.",
        )
        .unwrap();
        std::fs::write(
            memory_dir.join("file2.md"),
            "This file is about web development and JavaScript frameworks.",
        )
        .unwrap();
        // Non-md file should be ignored
        std::fs::write(memory_dir.join("file3.txt"), "This should be ignored.").unwrap();

        let db = MemoryDB::new(&db_path).unwrap();
        db.index_directory(&memory_dir).unwrap();

        let results = db.search("artificial intelligence", 10, None).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_memory_db_clone_works() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let db = MemoryDB::new(&db_path).unwrap();

        let test_file = dir.path().join("notes.md");
        std::fs::write(
            &test_file,
            "Content about cloning and testing database connections.",
        )
        .unwrap();
        db.index_file("notes.md", &test_file).unwrap();

        let db2 = db.clone();
        let results = db2.search("cloning", 10, None).unwrap();
        assert!(!results.is_empty());
    }
}
