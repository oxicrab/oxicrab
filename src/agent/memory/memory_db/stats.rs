use super::MemoryDB;
use super::search::MemoryHit;
use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct SearchDetails {
    pub query: String,
    pub search_type: String,
    pub result_count: usize,
    pub top_score: Option<f64>,
    pub source_keys: Vec<String>,
    pub timestamp: String,
}

#[derive(Debug, Clone)]
pub struct SearchStats {
    pub total_searches: u64,
    pub total_hits: u64,
    pub avg_results_per_search: f64,
}

#[derive(Debug, Clone)]
pub struct ComplexityTierStats {
    pub tier: String,
    pub count: u64,
    pub avg_score: f64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone)]
pub struct ComplexityForceCount {
    pub reason: String,
    pub count: u64,
}

#[derive(Debug, Clone)]
pub struct ComplexityEvent {
    pub timestamp: String,
    pub composite_score: f64,
    pub resolved_tier: String,
    pub resolved_model: Option<String>,
    pub forced: Option<String>,
    pub message_preview: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ComplexityStats {
    pub total_scored: u64,
    pub tier_counts: Vec<ComplexityTierStats>,
    pub force_counts: Vec<ComplexityForceCount>,
}

impl MemoryDB {
    /// Log a search query and the source keys it returned.
    pub fn log_search(
        &self,
        query: &str,
        search_type: &str,
        results: &[MemoryHit],
        top_score: Option<f64>,
        request_id: Option<&str>,
    ) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO memory_access_log (query, search_type, result_count, top_score, request_id)
             VALUES (?, ?, ?, ?, ?)",
            params![query, search_type, results.len() as i64, top_score, request_id],
        )?;
        let log_id = tx.last_insert_rowid();
        for hit in results {
            tx.execute(
                "INSERT INTO memory_search_hits (access_log_id, source_key) VALUES (?, ?)",
                params![log_id, hit.source_key],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Return provenance details for the most recent memory search.
    pub fn get_last_search_details(&self) -> Result<Option<SearchDetails>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;

        let mut stmt = conn.prepare(
            "SELECT id, query, search_type, result_count, top_score, created_at
             FROM memory_access_log ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query([])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };

        let log_id: i64 = row.get(0)?;
        let query: String = row.get(1)?;
        let search_type: String = row.get(2)?;
        let result_count: i64 = row.get(3)?;
        let top_score: Option<f64> = row.get(4)?;
        let created_at: String = row.get(5)?;

        let mut hit_stmt =
            conn.prepare("SELECT source_key FROM memory_search_hits WHERE access_log_id = ?")?;
        let source_keys: Vec<String> = hit_stmt
            .query_map([log_id], |r| r.get(0))?
            .filter_map(Result::ok)
            .collect();

        Ok(Some(SearchDetails {
            query,
            search_type,
            result_count: result_count as usize,
            top_score,
            source_keys,
            timestamp: created_at,
        }))
    }

    /// Count how many times a source key appeared in search results.
    pub fn get_source_hit_count(&self, source_key: &str) -> Result<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
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
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
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
        rows.map_err(|e| anyhow::anyhow!("failed to get entries missing embeddings: {e}"))
    }

    /// Get search log stats: total searches, total hits, unique queries.
    pub fn get_search_stats(&self) -> Result<SearchStats> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
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
            .unwrap_or_default();
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
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT source_key, COUNT(*) as hits FROM memory_search_hits
             GROUP BY source_key ORDER BY hits DESC LIMIT ?",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map([limit as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })?
            .collect();
        rows.map_err(|e| anyhow::anyhow!("failed to get top sources: {e}"))
    }

    /// Record a complexity routing decision for a message.
    #[allow(clippy::too_many_arguments)]
    pub fn record_complexity_event(
        &self,
        request_id: &str,
        composite_score: f64,
        resolved_tier: &str,
        resolved_model: Option<&str>,
        forced: Option<&str>,
        channel: Option<&str>,
        message_preview: &str,
    ) -> Result<()> {
        let preview = &message_preview[..message_preview
            .char_indices()
            .nth(80)
            .map_or(message_preview.len(), |(i, _)| i)];
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        conn.execute(
            "INSERT INTO complexity_routing_log
             (request_id, composite_score, resolved_tier, resolved_model, forced, channel, message_preview)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                request_id,
                composite_score,
                resolved_tier,
                resolved_model,
                forced,
                channel,
                preview,
            ],
        )?;
        Ok(())
    }

    /// Get complexity routing statistics for the given period.
    pub fn get_complexity_stats(&self, since_date: &str) -> Result<ComplexityStats> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;

        let since_datetime = format!("{since_date} 00:00:00");
        let total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM complexity_routing_log WHERE timestamp >= ?",
                [&since_datetime],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Per-tier stats with token correlation via request_id JOIN
        let mut stmt = conn.prepare(
            "SELECT c.resolved_tier,
                    COUNT(*) as cnt,
                    AVG(c.composite_score) as avg_score,
                    COALESCE(SUM(l.input_tokens + l.output_tokens), 0) as total_tokens
             FROM complexity_routing_log c
             LEFT JOIN llm_cost_log l ON c.request_id = l.request_id
             WHERE c.timestamp >= ?
             GROUP BY c.resolved_tier
             ORDER BY cnt DESC",
        )?;
        let tier_counts: Vec<ComplexityTierStats> = stmt
            .query_map([&since_datetime], |row| {
                Ok(ComplexityTierStats {
                    tier: row.get(0)?,
                    count: row.get::<_, i64>(1)? as u64,
                    avg_score: row.get(2)?,
                    total_tokens: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("tier stats query failed: {e}"))?;

        // Force override counts
        let mut stmt = conn.prepare(
            "SELECT forced, COUNT(*) as cnt
             FROM complexity_routing_log
             WHERE forced IS NOT NULL AND timestamp >= ?
             GROUP BY forced
             ORDER BY cnt DESC",
        )?;
        let force_counts: Vec<ComplexityForceCount> = stmt
            .query_map([&since_datetime], |row| {
                Ok(ComplexityForceCount {
                    reason: row.get(0)?,
                    count: row.get::<_, i64>(1)? as u64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("force counts query failed: {e}"))?;

        Ok(ComplexityStats {
            total_scored: total as u64,
            tier_counts,
            force_counts,
        })
    }

    /// Get recent complexity routing events for a given tier.
    pub fn get_recent_complexity_events(
        &self,
        tier: &str,
        limit: usize,
    ) -> Result<Vec<ComplexityEvent>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT timestamp, composite_score, resolved_tier, resolved_model, forced, message_preview
             FROM complexity_routing_log
             WHERE resolved_tier = ?
             ORDER BY timestamp DESC LIMIT ?",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map(params![tier, limit as i64], |row| {
                Ok(ComplexityEvent {
                    timestamp: row.get(0)?,
                    composite_score: row.get(1)?,
                    resolved_tier: row.get(2)?,
                    resolved_model: row.get(3)?,
                    forced: row.get(4)?,
                    message_preview: row.get(5)?,
                })
            })?
            .collect();
        rows.map_err(|e| anyhow::anyhow!("recent complexity events query failed: {e}"))
    }

    /// Purge complexity routing logs older than `days`. Returns number of rows deleted.
    pub fn purge_old_complexity_logs(&self, days: u32) -> Result<usize> {
        if days == 0 {
            return Ok(0);
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(days));
        let cutoff_str = cutoff.format("%Y-%m-%d %H:%M:%S").to_string();
        let deleted = conn.execute(
            "DELETE FROM complexity_routing_log WHERE timestamp < ?",
            [&cutoff_str],
        )?;
        Ok(deleted)
    }

    /// Purge search access logs older than `days`. Returns number of rows deleted.
    /// Also cleans up orphaned `memory_search_hits` referencing deleted logs.
    /// Both deletes run in a single transaction to avoid partial cleanup.
    pub fn purge_old_search_logs(&self, days: u32) -> Result<usize> {
        if days == 0 {
            return Ok(0);
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(days));
        let cutoff_str = cutoff.format("%Y-%m-%d %H:%M:%S").to_string();
        let tx = conn.transaction()?;
        // Delete orphaned hits first (FK has no CASCADE)
        tx.execute(
            "DELETE FROM memory_search_hits WHERE access_log_id IN (
                 SELECT id FROM memory_access_log WHERE created_at < ?
             )",
            [&cutoff_str],
        )?;
        let deleted = tx.execute(
            "DELETE FROM memory_access_log WHERE created_at < ?",
            [&cutoff_str],
        )?;
        tx.commit()?;
        Ok(deleted)
    }
}
