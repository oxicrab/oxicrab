use super::MemoryDB;
use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct CostSummaryRow {
    pub date: String,
    pub model: String,
    pub total_cents: f64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub call_count: i64,
}

impl MemoryDB {
    /// Record an LLM cost entry.
    #[allow(clippy::too_many_arguments)]
    pub fn record_cost(
        &self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_tokens: u64,
        cache_read_tokens: u64,
        cost_cents: f64,
        caller: &str,
        request_id: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        conn.execute(
            "INSERT INTO llm_cost_log
             (model, input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens, cost_cents, caller, request_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                model,
                input_tokens as i64,
                output_tokens as i64,
                cache_creation_tokens as i64,
                cache_read_tokens as i64,
                cost_cents,
                caller,
                request_id,
            ],
        )?;
        Ok(())
    }

    /// Get total cost in cents for a given date (YYYY-MM-DD).
    pub fn get_daily_cost(&self, date_str: &str) -> Result<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let pattern = format!("{}%", date_str);
        let total: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_cents), 0.0) FROM llm_cost_log WHERE timestamp LIKE ?",
                [&pattern],
                |row| row.get(0),
            )
            .unwrap_or_default();
        Ok(total)
    }

    /// Get cost summary grouped by date and model since a given date (YYYY-MM-DD).
    pub fn get_cost_summary(&self, since_date: &str) -> Result<Vec<CostSummaryRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT DATE(timestamp) as day, model,
                    SUM(cost_cents) as total_cents,
                    SUM(input_tokens) as total_input,
                    SUM(output_tokens) as total_output,
                    COUNT(*) as call_count
             FROM llm_cost_log
             WHERE DATE(timestamp) >= ?
             GROUP BY day, model
             ORDER BY day DESC, total_cents DESC",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map([since_date], |row| {
                Ok(CostSummaryRow {
                    date: row.get(0)?,
                    model: row.get(1)?,
                    total_cents: row.get(2)?,
                    total_input_tokens: row.get(3)?,
                    total_output_tokens: row.get(4)?,
                    call_count: row.get(5)?,
                })
            })?
            .collect();
        rows.map_err(|e| anyhow::anyhow!("failed to get cost summary: {}", e))
    }
}
