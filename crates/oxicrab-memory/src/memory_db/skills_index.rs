use super::MemoryDB;
use anyhow::{Result, anyhow};
use rusqlite::params;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct SkillIndexEntry {
    pub path: String,
    pub name: String,
    pub description: String,
    pub embedding: Vec<f32>,
    pub file_sha256: String,
    /// Identifier for the embedding model that produced this row's
    /// `embedding`. Used to bulk-invalidate when the model changes.
    /// Empty string means "unknown" (pre-migration #9 rows).
    pub embedding_model_id: String,
    pub use_count: u64,
    pub last_used_ms: Option<i64>,
    pub created_at_ms: i64,
    pub last_indexed_ms: i64,
}

fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for f in embedding {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

/// Decode a `f32` little-endian BLOB stored in `skills_index.embedding`.
/// Returns `Err` when the byte length is not a multiple of 4 (the BLOB
/// is corrupted; silently dropping trailing bytes would degrade search
/// quality without warning).
fn blob_to_embedding(blob: &[u8]) -> Result<Vec<f32>> {
    if !blob.len().is_multiple_of(4) {
        return Err(anyhow!(
            "skills_index embedding blob has non-multiple-of-4 length ({} bytes); refusing to decode",
            blob.len()
        ));
    }
    let mut out = Vec::with_capacity(blob.len() / 4);
    for chunk in blob.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(out)
}

impl MemoryDB {
    /// Insert or replace a skill index entry. Preserves `use_count` and
    /// `last_used_ms` from any prior row at the same path so reindexing
    /// does not erase usage telemetry.
    pub fn upsert_skill_index(&self, entry: &SkillIndexEntry) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO skills_index (
                path, name, description, embedding, file_sha256,
                embedding_model_id, use_count, last_used_ms, created_at_ms, last_indexed_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(path) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                embedding = excluded.embedding,
                file_sha256 = excluded.file_sha256,
                embedding_model_id = excluded.embedding_model_id,
                last_indexed_ms = excluded.last_indexed_ms",
            params![
                entry.path,
                entry.name,
                entry.description,
                embedding_to_blob(&entry.embedding),
                entry.file_sha256,
                entry.embedding_model_id,
                entry.use_count as i64,
                entry.last_used_ms,
                entry.created_at_ms,
                entry.last_indexed_ms,
            ],
        )?;
        Ok(())
    }

    /// Look up the indexed `(sha256, embedding_model_id)` for a skill
    /// path. Returns `None` when the skill has never been indexed.
    pub fn get_skill_index_meta(&self, path: &str) -> Result<Option<(String, String)>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare("SELECT file_sha256, embedding_model_id FROM skills_index WHERE path = ?1")?;
        let mut rows = stmt.query([path])?;
        if let Some(row) = rows.next()? {
            Ok(Some((row.get(0)?, row.get(1)?)))
        } else {
            Ok(None)
        }
    }

    /// Return all skill index entries (small table, full scan is fine).
    pub fn list_skill_index_entries(&self) -> Result<Vec<SkillIndexEntry>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT path, name, description, embedding, file_sha256,
                    embedding_model_id,
                    use_count, last_used_ms, created_at_ms, last_indexed_ms
               FROM skills_index",
        )?;
        let mut out = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let path: String = row.get(0)?;
            let embedding_blob: Vec<u8> = row.get(3)?;
            let embedding = match blob_to_embedding(&embedding_blob) {
                Ok(v) => v,
                Err(e) => {
                    warn!("skills_index: skipping {path}: {e}");
                    metrics::counter!("oxicrab_skill_index_blob_corrupt_total").increment(1);
                    continue;
                }
            };
            out.push(SkillIndexEntry {
                path,
                name: row.get(1)?,
                description: row.get(2)?,
                embedding,
                file_sha256: row.get(4)?,
                embedding_model_id: row.get(5)?,
                use_count: row.get::<_, i64>(6)? as u64,
                last_used_ms: row.get(7)?,
                created_at_ms: row.get(8)?,
                last_indexed_ms: row.get(9)?,
            });
        }
        Ok(out)
    }

    /// Drop every row whose `embedding_model_id` differs from the
    /// supplied id. Returns the number of rows removed. Called by
    /// `SkillIndex::rebuild` when the operator-configured model
    /// changes so the next pass re-embeds everything fresh.
    ///
    /// Refuses an empty `current_model` to prevent a misconfigured
    /// (unset) embedding model id from silently wiping the entire
    /// index — every existing row would be different from `""` if any
    /// past embedding produced a non-empty id, and pre-migration-#9
    /// rows that default to `''` would get spared while real data is
    /// dropped. Either case is a foot-gun; require an explicit value.
    pub fn invalidate_skill_index_for_model(&self, current_model: &str) -> Result<u64> {
        if current_model.is_empty() {
            return Err(anyhow!(
                "invalidate_skill_index_for_model: refusing to invalidate against empty model id"
            ));
        }
        let conn = self.lock_conn()?;
        let n = conn.execute(
            "DELETE FROM skills_index WHERE embedding_model_id != ?1",
            params![current_model],
        )?;
        Ok(n as u64)
    }

    /// Bump `use_count` and update `last_used_ms` when a skill is selected
    /// for system-prompt injection.
    pub fn record_skill_use(&self, path: &str, now_ms: i64) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE skills_index
                SET use_count = use_count + 1,
                    last_used_ms = ?1
              WHERE path = ?2",
            params![now_ms, path],
        )?;
        Ok(())
    }

    /// Delete index entries whose `path` is not in the provided set.
    /// Returns the number of entries removed. Used by hygiene when a
    /// skill file is removed from disk.
    pub fn prune_skill_index(&self, live_paths: &std::collections::HashSet<String>) -> Result<u64> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare("SELECT path FROM skills_index")?;
        let mut to_drop = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let path: String = row.get(0)?;
            if !live_paths.contains(&path) {
                to_drop.push(path);
            }
        }
        for path in &to_drop {
            conn.execute("DELETE FROM skills_index WHERE path = ?1", [path])?;
        }
        Ok(to_drop.len() as u64)
    }

    /// Delete entries last used at-or-before `max_age_ms` ago whose
    /// `use_count` is below `min_uses`. Returns paths removed. Used by
    /// hygiene.
    ///
    /// The boundary is inclusive (`<=`) so a skill exactly `max_age_ms`
    /// old is treated as "older than the window" — semantically what
    /// callers expect for a TTL like "30 days".
    pub fn prune_unused_skill_index(
        &self,
        now_ms: i64,
        max_age_ms: i64,
        min_uses: u64,
    ) -> Result<Vec<String>> {
        let conn = self.lock_conn()?;
        let cutoff = now_ms - max_age_ms;
        let mut stmt = conn.prepare(
            "SELECT path FROM skills_index
              WHERE use_count < ?1
                AND created_at_ms <= ?2
                AND (last_used_ms IS NULL OR last_used_ms <= ?2)",
        )?;
        let mut to_drop = Vec::new();
        let mut rows = stmt.query(params![min_uses as i64, cutoff])?;
        while let Some(row) = rows.next()? {
            to_drop.push(row.get::<_, String>(0)?);
        }
        for path in &to_drop {
            conn.execute("DELETE FROM skills_index WHERE path = ?1", [path])?;
        }
        Ok(to_drop)
    }
}
