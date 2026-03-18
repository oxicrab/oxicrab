use super::MemoryDB;
use super::recency_decay;
use anyhow::Result;
use chrono::Utc;
use oxicrab_core::config::schema::FusionStrategy;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct MemoryHit {
    pub source_key: String,
    pub content: String,
}

/// Maximum number of unique terms used in FTS queries
pub(super) const MAX_FTS_TERMS: usize = 16;

impl MemoryDB {
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
        use crate::embeddings::cosine_similarity;

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
                let conn = self.lock_conn()?;
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
                            .map(|dt| dt.with_timezone(&Utc))
                            .or_else(|_| {
                                // Fallback for entries with SQLite datetime('now') format
                                chrono::NaiveDateTime::parse_from_str(
                                    &created_at,
                                    "%Y-%m-%d %H:%M:%S",
                                )
                                .map(|ndt| ndt.and_utc())
                            })
                            .map_or(0.0, |dt| (now - dt).num_seconds() as f64 / 86400.0);
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
                let fts_absent_rank = fts_ranked.len().max(1) + 1;

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
                let vec_absent_rank = vec_ranked.len().max(1) + 1;

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

        if let Err(e) = self.log_search(query_text, "hybrid", &hits, top_score, None) {
            debug!("failed to log hybrid search: {}", e);
        }

        Ok(hits)
    }

    /// List all source keys in the database.
    pub fn list_source_keys(&self) -> Result<Vec<String>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare("SELECT source_key FROM memory_sources")?;
        let keys: Result<Vec<_>, _> = stmt.query_map([], |row| row.get(0))?.collect();
        keys.map_err(|e| anyhow::anyhow!("Failed to list source keys: {e}"))
    }

    /// List source keys matching the `daily:` prefix.
    /// More efficient than `list_source_keys()` + client-side filtering for
    /// group-mode daily note exclusion.
    pub fn list_daily_source_keys(&self) -> Result<Vec<String>> {
        let conn = self.lock_conn()?;
        let mut stmt =
            conn.prepare("SELECT source_key FROM memory_sources WHERE source_key LIKE 'daily:%'")?;
        let keys: Result<Vec<_>, _> = stmt.query_map([], |row| row.get(0))?.collect();
        keys.map_err(|e| anyhow::anyhow!("failed to list daily source keys: {e}"))
    }

    pub fn search(
        &self,
        query_text: &str,
        limit: usize,
        exclude_sources: Option<&std::collections::HashSet<String>>,
    ) -> Result<Vec<MemoryHit>> {
        let hits = self.search_inner(query_text, limit, exclude_sources)?;
        // Log search asynchronously (best-effort, don't fail the search)
        if let Err(e) = self.log_search(query_text, "keyword", &hits, None, None) {
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
        let conn = self.lock_conn()?;

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
        let escaped: String = query_text
            .trim()
            .chars()
            .take(200)
            .flat_map(|c| match c {
                '%' => vec!['\\', '%'],
                '_' => vec!['\\', '_'],
                '\\' => vec!['\\', '\\'],
                other => vec![other],
            })
            .collect();
        let like = format!("%{escaped}%");
        let mut stmt = conn.prepare(
            "SELECT source_key, content
            FROM memory_entries
            WHERE content LIKE ? ESCAPE '\\'
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

pub(super) fn fts_query(text: &str) -> String {
    use std::sync::LazyLock;
    static WORDS_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"[A-Za-z0-9_]{2,}").expect("words regex"));
    let re = &*WORDS_RE;
    let mut seen = std::collections::HashSet::new();
    let mut unique = Vec::new();

    for m in re.find_iter(text) {
        let low = m.as_str().to_lowercase();
        if seen.insert(low.clone()) {
            unique.push(low);
        }
        if unique.len() >= MAX_FTS_TERMS {
            break;
        }
    }

    if unique.is_empty() {
        return String::new();
    }

    // Double-quote each term to prevent FTS5 operator injection
    // (e.g. user searching for "NOT important" won't trigger NOT operator)
    unique
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
}
