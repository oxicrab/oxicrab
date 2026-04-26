use super::MemoryDB;
use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct DlqEntry {
    pub id: i64,
    pub job_id: String,
    pub job_name: String,
    pub payload: String,
    pub error_message: String,
    pub failed_at: String,
    pub retry_count: i64,
    pub status: String,
}

/// Scrub a DLQ payload: try to parse as JSON and walk the tree
/// (catches `metadata.api_key` etc.); fall back to text scrub
/// (catches `Authorization: Bearer …` lines) when not valid JSON.
fn scrub_dlq_payload(payload: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
        let scrubbed = oxicrab_safety::scrub_credentials_in_json(&value);
        return scrubbed.to_string();
    }
    oxicrab_safety::scrub_credentials_in_text(payload)
}

impl MemoryDB {
    pub fn insert_dlq_entry(
        &self,
        job_id: &str,
        job_name: &str,
        payload: &str,
        error_message: &str,
    ) -> Result<i64> {
        // Scrub credentials defensively — DLQ payload contains the full
        // inbound message JSON, error_message echoes whatever the
        // upstream failure produced. Callers don't need to remember to
        // scrub themselves.
        let payload = scrub_dlq_payload(payload);
        let error_message = oxicrab_safety::scrub_credentials_in_text(error_message);
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO scheduled_task_dlq (job_id, job_name, payload, error_message)
             VALUES (?1, ?2, ?3, ?4)",
            params![job_id, job_name, payload, error_message],
        )?;
        let id = tx.last_insert_rowid();

        // Auto-purge: keep only 100 most recent entries
        tx.execute(
            "DELETE FROM scheduled_task_dlq WHERE id NOT IN (
                SELECT id FROM scheduled_task_dlq ORDER BY id DESC LIMIT 100
            )",
            [],
        )?;
        tx.commit()?;

        Ok(id)
    }

    pub fn list_dlq_entries(&self, status_filter: Option<&str>) -> Result<Vec<DlqEntry>> {
        let conn = self.lock_conn()?;
        let (sql, filter_params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(
            status,
        ) =
            status_filter
        {
            (
                    "SELECT id, job_id, job_name, payload, error_message, failed_at, retry_count, status
                     FROM scheduled_task_dlq WHERE status = ?1 ORDER BY id DESC",
                    vec![Box::new(status.to_string())],
                )
        } else {
            (
                    "SELECT id, job_id, job_name, payload, error_message, failed_at, retry_count, status
                     FROM scheduled_task_dlq ORDER BY id DESC",
                    vec![],
                )
        };

        let mut stmt = conn.prepare(sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            filter_params.iter().map(AsRef::as_ref).collect();
        let rows = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok(DlqEntry {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    job_name: row.get(2)?,
                    payload: row.get(3)?,
                    error_message: row.get(4)?,
                    failed_at: row.get(5)?,
                    retry_count: row.get(6)?,
                    status: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    pub fn update_dlq_status(&self, id: i64, new_status: &str) -> Result<bool> {
        let conn = self.lock_conn()?;
        let updated = conn.execute(
            "UPDATE scheduled_task_dlq SET status = ?1 WHERE id = ?2",
            params![new_status, id],
        )?;
        Ok(updated > 0)
    }

    /// Maximum retry count tracked in the DLQ. The cron tool's
    /// max-5 check is the LLM-facing gate; this is a hard ceiling
    /// on the database row to bound long-tail accumulation when an
    /// operator scripts replays directly.
    const MAX_DLQ_RETRIES: i64 = 16;

    pub fn increment_dlq_retry(&self, id: i64) -> Result<bool> {
        let conn = self.lock_conn()?;
        let updated = conn.execute(
            "UPDATE scheduled_task_dlq
                SET retry_count = retry_count + 1
              WHERE id = ?1 AND retry_count < ?2",
            params![id, Self::MAX_DLQ_RETRIES],
        )?;
        Ok(updated > 0)
    }

    pub fn clear_dlq(&self, status_filter: Option<&str>) -> Result<usize> {
        let conn = self.lock_conn()?;
        let deleted = if let Some(status) = status_filter {
            conn.execute(
                "DELETE FROM scheduled_task_dlq WHERE status = ?1",
                params![status],
            )?
        } else {
            conn.execute("DELETE FROM scheduled_task_dlq", [])?
        };
        Ok(deleted)
    }
}
