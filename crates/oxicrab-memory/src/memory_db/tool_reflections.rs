use super::MemoryDB;
use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct ReflectionRecord {
    pub request_id: String,
    pub tool_name: String,
    pub action: Option<String>,
    pub attempt_number: u32,
    pub error_excerpt: String,
    pub hypothesis: String,
    pub retry_strategy: String,
    pub next_outcome: Option<String>,
    pub created_at_ms: i64,
}

impl MemoryDB {
    /// Persist a single tool-reflection record. `INSERT OR IGNORE`
    /// against the unique index on
    /// (request_id, tool_name, action, attempt_number) so a retry
    /// path logging the same reflection twice doesn't leave a
    /// duplicate row.
    pub fn insert_tool_reflection(&self, rec: &ReflectionRecord) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO tool_reflections (
                request_id, tool_name, action, attempt_number, error_excerpt,
                hypothesis, retry_strategy, next_outcome, created_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                rec.request_id,
                rec.tool_name,
                rec.action,
                rec.attempt_number as i64,
                rec.error_excerpt,
                rec.hypothesis,
                rec.retry_strategy,
                rec.next_outcome,
                rec.created_at_ms,
            ],
        )?;
        Ok(())
    }

    /// Update the next_outcome for the most recent reflection on a given
    /// (request_id, tool, action) tuple. Used to record whether the retry
    /// succeeded after the reflection was injected.
    ///
    /// Action matching uses an explicit branch on `Option`: SQL's
    /// `=` does not match NULL on either side, so a single
    /// `(action IS ?4 OR action = ?4)` clause is fragile and depends on
    /// the bound type round-trip — use two prepared statements instead.
    pub fn update_reflection_outcome(
        &self,
        request_id: &str,
        tool_name: &str,
        action: Option<&str>,
        outcome: &str,
    ) -> Result<()> {
        let conn = self.lock_conn()?;
        if let Some(action) = action {
            conn.execute(
                "UPDATE tool_reflections
                    SET next_outcome = ?1
                  WHERE id = (
                    SELECT id FROM tool_reflections
                     WHERE request_id = ?2
                       AND tool_name = ?3
                       AND action = ?4
                       AND next_outcome IS NULL
                     ORDER BY id DESC LIMIT 1
                  )",
                params![outcome, request_id, tool_name, action],
            )?;
        } else {
            conn.execute(
                "UPDATE tool_reflections
                    SET next_outcome = ?1
                  WHERE id = (
                    SELECT id FROM tool_reflections
                     WHERE request_id = ?2
                       AND tool_name = ?3
                       AND action IS NULL
                       AND next_outcome IS NULL
                     ORDER BY id DESC LIMIT 1
                  )",
                params![outcome, request_id, tool_name],
            )?;
        }
        Ok(())
    }

    /// Count reflections persisted for a given request id.
    /// Used by tests and `cli stats`.
    pub fn count_reflections_for_request(&self, request_id: &str) -> Result<u64> {
        let conn = self.lock_conn()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tool_reflections WHERE request_id = ?1",
            [request_id],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Purge `tool_reflections` rows older than `days`. Returns the
    /// number of rows deleted. Called from `run_hygiene` so the table
    /// stays bounded under long-running deployments with reflection
    /// enabled (without hygiene, every retry persists a row that's
    /// never reaped).
    pub fn purge_old_tool_reflections(&self, days: u32) -> Result<usize> {
        if days == 0 {
            return Ok(0);
        }
        let conn = self.lock_conn()?;
        let cutoff_ms = chrono::Utc::now().timestamp_millis() - i64::from(days) * 86_400_000;
        let deleted = conn.execute(
            "DELETE FROM tool_reflections WHERE created_at_ms < ?1",
            rusqlite::params![cutoff_ms],
        )?;
        Ok(deleted)
    }

    /// Aggregate reflection statistics per `(tool, action)` over the
    /// last `days_back` days. Used by `oxicrab stats reflections` and
    /// the hygiene job to surface tools where retries consistently
    /// fail (high `errors` / `total` ratio with `total >= min_samples`).
    pub fn reflection_stats(
        &self,
        days_back: u32,
        min_samples: u64,
    ) -> Result<Vec<ReflectionStatRow>> {
        let conn = self.lock_conn()?;
        let cutoff_ms = chrono::Utc::now().timestamp_millis() - i64::from(days_back) * 86_400_000;
        // Group on `action` directly — a NULL action (single-purpose
        // tool) and an empty-string action (action-based tool with an
        // empty value) are semantically distinct. `COALESCE(action,'')`
        // would conflate them.
        let mut stmt = conn.prepare(
            "SELECT tool_name,
                    action,
                    COUNT(*) AS total,
                    SUM(CASE WHEN next_outcome = 'success' THEN 1 ELSE 0 END) AS successes,
                    SUM(CASE WHEN next_outcome = 'error' THEN 1 ELSE 0 END) AS errors,
                    SUM(CASE WHEN next_outcome IS NULL THEN 1 ELSE 0 END) AS pending
               FROM tool_reflections
              WHERE created_at_ms >= ?1
              GROUP BY tool_name, action
             HAVING total >= ?2
              ORDER BY total DESC, tool_name ASC",
        )?;
        let mut out = Vec::new();
        let mut rows = stmt.query(rusqlite::params![cutoff_ms, min_samples as i64])?;
        while let Some(row) = rows.next()? {
            out.push(ReflectionStatRow {
                tool_name: row.get(0)?,
                action: row.get::<_, Option<String>>(1)?,
                total: row.get::<_, i64>(2)? as u64,
                successes: row.get::<_, i64>(3)? as u64,
                errors: row.get::<_, i64>(4)? as u64,
                pending: row.get::<_, i64>(5)? as u64,
            });
        }
        Ok(out)
    }
}

/// One aggregated row returned by `reflection_stats`.
#[derive(Debug, Clone)]
pub struct ReflectionStatRow {
    pub tool_name: String,
    pub action: Option<String>,
    pub total: u64,
    pub successes: u64,
    pub errors: u64,
    pub pending: u64,
}

impl ReflectionStatRow {
    /// `errors / (errors + successes)` — pending rows are excluded.
    /// Returns `None` when no resolved samples exist.
    #[must_use]
    pub fn failure_rate(&self) -> Option<f64> {
        let resolved = self.successes + self.errors;
        if resolved == 0 {
            None
        } else {
            Some(self.errors as f64 / resolved as f64)
        }
    }
}
