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
    /// Persist a single tool-reflection record.
    pub fn insert_tool_reflection(&self, rec: &ReflectionRecord) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO tool_reflections (
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
}
