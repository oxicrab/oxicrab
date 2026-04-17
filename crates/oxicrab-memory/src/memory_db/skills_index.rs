use super::MemoryDB;
use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct SkillIndexEntry {
    pub path: String,
    pub name: String,
    pub description: String,
    pub embedding: Vec<f32>,
    pub file_sha256: String,
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

fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    let mut out = Vec::with_capacity(blob.len() / 4);
    for chunk in blob.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    out
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
                use_count, last_used_ms, created_at_ms, last_indexed_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(path) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                embedding = excluded.embedding,
                file_sha256 = excluded.file_sha256,
                last_indexed_ms = excluded.last_indexed_ms",
            params![
                entry.path,
                entry.name,
                entry.description,
                embedding_to_blob(&entry.embedding),
                entry.file_sha256,
                entry.use_count as i64,
                entry.last_used_ms,
                entry.created_at_ms,
                entry.last_indexed_ms,
            ],
        )?;
        Ok(())
    }

    /// Look up the indexed sha256 for a skill path. Returns `None` when
    /// the skill has never been indexed.
    pub fn get_skill_index_sha(&self, path: &str) -> Result<Option<String>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare("SELECT file_sha256 FROM skills_index WHERE path = ?1")?;
        let mut rows = stmt.query([path])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Return all skill index entries (small table, full scan is fine).
    pub fn list_skill_index_entries(&self) -> Result<Vec<SkillIndexEntry>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT path, name, description, embedding, file_sha256,
                    use_count, last_used_ms, created_at_ms, last_indexed_ms
               FROM skills_index",
        )?;
        let mut out = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let embedding_blob: Vec<u8> = row.get(3)?;
            out.push(SkillIndexEntry {
                path: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                embedding: blob_to_embedding(&embedding_blob),
                file_sha256: row.get(4)?,
                use_count: row.get::<_, i64>(5)? as u64,
                last_used_ms: row.get(6)?,
                created_at_ms: row.get(7)?,
                last_indexed_ms: row.get(8)?,
            });
        }
        Ok(out)
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

    /// Delete entries last used > `max_age_ms` ago whose `use_count`
    /// is below `min_uses`. Returns paths removed. Used by hygiene.
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
                AND created_at_ms < ?2
                AND (last_used_ms IS NULL OR last_used_ms < ?2)",
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
