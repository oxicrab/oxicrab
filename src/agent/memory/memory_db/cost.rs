use super::MemoryDB;
use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct TokenSummaryRow {
    pub date: String,
    pub model: String,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cache_creation_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub call_count: i64,
}

impl MemoryDB {
    /// Record LLM token usage.
    #[allow(clippy::too_many_arguments)]
    pub fn record_tokens(
        &self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_tokens: u64,
        cache_read_tokens: u64,
        caller: &str,
        request_id: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        conn.execute(
            "INSERT INTO llm_cost_log
             (model, input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens, cost_cents, caller, request_id)
             VALUES (?, ?, ?, ?, ?, 0.0, ?, ?)",
            params![
                model,
                input_tokens as i64,
                output_tokens as i64,
                cache_creation_tokens as i64,
                cache_read_tokens as i64,
                caller,
                request_id,
            ],
        )?;
        Ok(())
    }

    /// Purge cost log entries older than `days`. Returns number of rows deleted.
    pub fn purge_old_cost_logs(&self, days: u32) -> Result<usize> {
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
            "DELETE FROM llm_cost_log WHERE timestamp < ?",
            [&cutoff_str],
        )?;
        Ok(deleted)
    }

    /// Get token usage summary grouped by date and model since a given date (YYYY-MM-DD).
    pub fn get_token_summary(&self, since_date: &str) -> Result<Vec<TokenSummaryRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT DATE(timestamp) as day, model,
                    SUM(input_tokens) as total_input,
                    SUM(output_tokens) as total_output,
                    COALESCE(SUM(cache_creation_tokens), 0) as total_cache_creation,
                    COALESCE(SUM(cache_read_tokens), 0) as total_cache_read,
                    COUNT(*) as call_count
             FROM llm_cost_log
             WHERE DATE(timestamp) >= ?
             GROUP BY day, model
             ORDER BY day DESC, total_input DESC",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map([since_date], |row| {
                Ok(TokenSummaryRow {
                    date: row.get(0)?,
                    model: row.get(1)?,
                    total_input_tokens: row.get(2)?,
                    total_output_tokens: row.get(3)?,
                    total_cache_creation_tokens: row.get(4)?,
                    total_cache_read_tokens: row.get(5)?,
                    call_count: row.get(6)?,
                })
            })?
            .collect();
        rows.map_err(|e| anyhow::anyhow!("failed to get token summary: {e}"))
    }
}
