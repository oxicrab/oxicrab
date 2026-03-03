use super::MemoryDB;
use super::search::MemoryHit;
use anyhow::Result;
use rusqlite::params;

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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        conn.execute(
            "INSERT INTO memory_access_log (query, search_type, result_count, top_score, request_id)
             VALUES (?, ?, ?, ?, ?)",
            params![query, search_type, results.len() as i64, top_score, request_id],
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
        request_id: Option<&str>,
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
             (event_type, intent_method, semantic_score, detection_layer, message_preview, request_id)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                event_type,
                intent_method,
                semantic_score,
                detection_layer,
                preview,
                request_id,
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
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
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

    /// Purge search access logs older than `days`. Returns number of rows deleted.
    /// Also cleans up orphaned `memory_search_hits` referencing deleted logs.
    pub fn purge_old_search_logs(&self, days: u32) -> Result<usize> {
        if days == 0 {
            return Ok(0);
        }
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
}
