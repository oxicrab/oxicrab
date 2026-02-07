use anyhow::Result;
use chrono::Utc;
use regex::Regex;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tracing::debug;

#[derive(Debug, Clone)]
pub struct MemoryHit {
    pub source_key: String,
    pub content: String,
}

pub struct MemoryDB {
    db_path: PathBuf,
    has_fts: bool,
}

impl MemoryDB {
    pub fn new(db_path: impl AsRef<Path>) -> Result<Self> {
        let db_path = db_path.as_ref();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut db = Self {
            db_path: db_path.to_path_buf(),
            has_fts: false,
        };

        db.ensure_schema()?;
        Ok(db)
    }

    fn connect(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)?;
        // Use execute_batch for PRAGMA statements that might return values
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=3000;",
        )?;
        Ok(conn)
    }

    fn ensure_schema(&mut self) -> Result<()> {
        let conn = self.connect()?;

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

        // Try to create FTS5 virtual table
        match conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts
            USING fts5(
                content,
                source_key,
                content='memory_entries',
                content_rowid='id'
            )",
            [],
        ) {
            Ok(_) => {
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
            }
            Err(_) => {
                self.has_fts = false;
                debug!("FTS5 not available; falling back to LIKE");
            }
        }

        Ok(())
    }

    fn get_mtime_ns(path: &Path) -> i64 {
        path.metadata()
            .and_then(|m| {
                m.modified()
                    .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as i64)
            })
            .unwrap_or(0)
    }

    pub fn index_file(&self, source_key: &str, path: &Path) -> Result<()> {
        let mtime_ns = Self::get_mtime_ns(path);
        let now = Utc::now().to_rfc3339();

        let conn = self.connect()?;

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
        let conn = self.connect()?;

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

fn _utc_now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn hash_text(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hex::encode(hasher.finalize())
}

fn split_into_chunks(text: &str) -> Vec<String> {
    let re = Regex::new(r"\n\s*\n+").unwrap();
    let raw: Vec<&str> = re.split(text.trim()).collect();
    let mut chunks = Vec::new();

    for part in raw {
        let p = part.trim();
        if p.is_empty() || p.len() < 12 {
            continue;
        }
        let chunk = if p.len() > 1200 {
            p[..1200].to_string()
        } else {
            p.to_string()
        };
        chunks.push(chunk);
    }

    chunks
}

fn fts_query(text: &str) -> String {
    let re = Regex::new(r"[A-Za-z0-9_]{2,}").unwrap();
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
        if unique.len() >= 16 {
            break;
        }
    }

    unique.join(" OR ")
}
