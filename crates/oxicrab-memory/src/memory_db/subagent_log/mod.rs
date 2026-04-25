use super::MemoryDB;
use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct SubagentLogEntry {
    pub id: i64,
    pub task_id: String,
    pub timestamp: String,
    pub event_type: String,
    pub content: String,
    pub metadata: Option<String>,
}

impl MemoryDB {
    /// Insert a subagent log entry. Content and metadata go through
    /// the credential scrubber before persistence so a subagent that
    /// echoes a tool's `Authorization: Bearer …` or `?api_key=…` into
    /// its log doesn't leave the secret on disk.
    pub fn insert_subagent_log(
        &self,
        task_id: &str,
        event_type: &str,
        content: &str,
        metadata: Option<&str>,
    ) -> Result<()> {
        let content = oxicrab_safety::scrub_credentials_in_text(content);
        let metadata = metadata.map(|m| {
            // Try JSON first; fall back to text. Mirrors `scrub_dlq_payload`.
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(m) {
                oxicrab_safety::scrub_credentials_in_json(&v).to_string()
            } else {
                oxicrab_safety::scrub_credentials_in_text(m)
            }
        });
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO subagent_logs (task_id, event_type, content, metadata)
             VALUES (?1, ?2, ?3, ?4)",
            params![task_id, event_type, content, metadata],
        )?;
        Ok(())
    }

    /// List all log entries for a task, ordered by id.
    pub fn list_subagent_logs(&self, task_id: &str) -> Result<Vec<SubagentLogEntry>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, timestamp, event_type, content, metadata
             FROM subagent_logs WHERE task_id = ?1 ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![task_id], |row| {
                Ok(SubagentLogEntry {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    timestamp: row.get(2)?,
                    event_type: row.get(3)?,
                    content: row.get(4)?,
                    metadata: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// List distinct `task_ids`, most recent first.
    pub fn list_recent_subagent_tasks(&self, limit: usize) -> Result<Vec<String>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT task_id FROM subagent_logs
             GROUP BY task_id ORDER BY MAX(id) DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Delete logs for tasks beyond the most recent N. Returns count deleted.
    pub fn purge_old_subagent_logs(&self, keep_tasks: usize) -> Result<usize> {
        let conn = self.lock_conn()?;
        let deleted = conn.execute(
            "DELETE FROM subagent_logs WHERE task_id NOT IN (
                SELECT task_id FROM subagent_logs
                GROUP BY task_id ORDER BY MAX(id) DESC LIMIT ?1
            )",
            params![keep_tasks as i64],
        )?;
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests;
