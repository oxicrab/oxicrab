//! Embedding-indexed skill retrieval.
//!
//! Track 2a of the self-improvement design. Maintains an embedding
//! index of all skill descriptions in the `skills_index` `SQLite` table
//! and provides `top_k_for_query` for retrieval.
//!
//! Indexing is lazy and incremental: on rebuild, files whose sha256
//! matches the stored value are skipped, so re-indexing is a no-op when
//! nothing has changed. Usage counters are preserved across re-indexes.

use crate::agent::memory::memory_db::{MemoryDB, SkillIndexEntry};
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, warn};
use walkdir::WalkDir;

#[cfg(feature = "embeddings")]
use crate::agent::memory::embeddings::{EmbeddingService, cosine_similarity};

/// Maximum number of skills returned by `top_k_for_query` regardless
/// of caller request. Matches the design doc's
/// `agents.defaults.skills.maxSystemPromptSkills` default.
pub const DEFAULT_TOP_K_CAP: usize = 5;

#[derive(Debug, Clone)]
pub struct ScoredSkill {
    pub path: String,
    pub name: String,
    pub description: String,
    pub score: f32,
}

pub struct SkillIndex {
    db: Arc<MemoryDB>,
    workspace_skills: PathBuf,
    builtin_skills: Option<PathBuf>,
}

impl SkillIndex {
    pub fn new(
        db: Arc<MemoryDB>,
        workspace_skills: PathBuf,
        builtin_skills: Option<PathBuf>,
    ) -> Self {
        Self {
            db,
            workspace_skills,
            builtin_skills,
        }
    }

    /// Rebuild the index from disk. For each skill file, compute sha256
    /// and re-embed only when the content has changed since the last
    /// index. Returns the number of entries (re)indexed.
    #[cfg(feature = "embeddings")]
    pub fn rebuild(&self, embeddings: &EmbeddingService) -> Result<u64> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut indexed = 0_u64;
        let mut live_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (path, name) in self.list_skill_files() {
            let path_str = path.to_string_lossy().to_string();
            live_paths.insert(path_str.clone());

            let Ok(content) = std::fs::read_to_string(&path) else {
                warn!("skills_index: unable to read {}", path.display());
                continue;
            };
            let sha = sha256_hex(&content);
            if let Ok(Some(existing)) = self.db.get_skill_index_sha(&path_str)
                && existing == sha
            {
                continue;
            }

            let description = extract_description(&content).unwrap_or_else(|| name.clone());
            let embedding = match embeddings.embed_texts(&[&description]) {
                Ok(mut v) => v.pop().unwrap_or_default(),
                Err(e) => {
                    warn!("skills_index: embedding failed for {}: {e}", path.display());
                    continue;
                }
            };

            let entry = SkillIndexEntry {
                path: path_str,
                name,
                description,
                embedding,
                file_sha256: sha,
                use_count: 0,
                last_used_ms: None,
                created_at_ms: now_ms,
                last_indexed_ms: now_ms,
            };
            if let Err(e) = self.db.upsert_skill_index(&entry) {
                warn!("skills_index: upsert failed for {}: {e}", entry.path);
                continue;
            }
            indexed += 1;
        }

        // Drop entries for files that were removed from disk.
        match self.db.prune_skill_index(&live_paths) {
            Ok(n) if n > 0 => debug!("skills_index: pruned {n} dead path(s)"),
            Ok(_) => {}
            Err(e) => warn!("skills_index: prune failed: {e}"),
        }

        debug!("skills_index: rebuilt {indexed} entry/entries");
        Ok(indexed)
    }

    /// Stub used when `embeddings` feature is disabled. Returns 0; the
    /// index becomes a no-op and `top_k_for_query` returns nothing.
    #[cfg(not(feature = "embeddings"))]
    pub fn rebuild_no_embeddings(&self) -> Result<u64> {
        Ok(0)
    }

    /// Return the top-k skills with highest cosine similarity to the
    /// query embedding. Bumps the per-skill `use_count` for hits.
    #[cfg(feature = "embeddings")]
    pub fn top_k_for_query(
        &self,
        embeddings: &EmbeddingService,
        query: &str,
        k: usize,
    ) -> Result<Vec<ScoredSkill>> {
        let k = k.min(DEFAULT_TOP_K_CAP);
        if k == 0 {
            return Ok(Vec::new());
        }
        let query_emb = embeddings.embed_query(query)?;
        let mut entries = self.db.list_skill_index_entries()?;
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        let mut scored: Vec<(f32, SkillIndexEntry)> = entries
            .drain(..)
            .filter_map(|e| {
                if e.embedding.len() != query_emb.len() {
                    return None;
                }
                let s = cosine_similarity(&query_emb, &e.embedding);
                Some((s, e))
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);

        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut out = Vec::with_capacity(scored.len());
        for (score, entry) in scored {
            if let Err(e) = self.db.record_skill_use(&entry.path, now_ms) {
                debug!(
                    "skills_index: record_skill_use failed for {}: {e}",
                    entry.path
                );
            }
            out.push(ScoredSkill {
                path: entry.path,
                name: entry.name,
                description: entry.description,
                score,
            });
        }
        Ok(out)
    }

    /// Hygiene: drop entries that have not been used in `max_age_days`
    /// and have a `use_count` below `min_uses`. Returns paths removed.
    pub fn prune_unused(&self, max_age_days: u32, min_uses: u64) -> Result<Vec<String>> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let max_age_ms = i64::from(max_age_days) * 86_400_000;
        self.db
            .prune_unused_skill_index(now_ms, max_age_ms, min_uses)
    }

    fn list_skill_files(&self) -> Vec<(PathBuf, String)> {
        let mut out = Vec::new();
        for root in [Some(&self.workspace_skills), self.builtin_skills.as_ref()]
            .into_iter()
            .flatten()
        {
            if !root.exists() {
                continue;
            }
            for entry in WalkDir::new(root)
                .max_depth(1)
                .follow_links(false)
                .into_iter()
                .flatten()
            {
                if entry.file_type().is_dir() && entry.path() != root.as_path() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let skill_file = entry.path().join(format!("{name}.md"));
                    if skill_file.exists() {
                        out.push((skill_file, name));
                    }
                }
            }
        }
        out
    }
}

fn sha256_hex(content: &str) -> String {
    let mut h = Sha256::new();
    h.update(content.as_bytes());
    hex::encode(h.finalize())
}

/// Extract the first non-empty line under `description:` in the
/// frontmatter, or fall back to the first non-blank, non-`#` markdown
/// line. Used so the embedding represents the skill's purpose, not the
/// file content.
fn extract_description(content: &str) -> Option<String> {
    if let Some(rest) = content.strip_prefix("---")
        && let Some(end_idx) = rest.find("\n---\n")
    {
        let frontmatter = &rest[..end_idx];
        for line in frontmatter.lines() {
            if let Some(value) = line.trim().strip_prefix("description:") {
                let v = value.trim().trim_matches('"').trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with("---") {
            return Some(trimmed.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_description_from_frontmatter() {
        let content = "---\nname: foo\ndescription: A handy tool.\n---\n\n# Foo\n";
        assert_eq!(
            extract_description(content),
            Some("A handy tool.".to_string())
        );
    }

    #[test]
    fn falls_back_to_first_body_line() {
        let content = "# Title\n\nFirst body line.\nSecond.";
        assert_eq!(
            extract_description(content),
            Some("First body line.".to_string())
        );
    }

    #[test]
    fn sha256_is_stable() {
        let a = sha256_hex("hello");
        let b = sha256_hex("hello");
        assert_eq!(a, b);
        assert_ne!(a, sha256_hex("world"));
    }
}
